use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use actix_web::{HttpResponse, Responder, post, web};
use serde::Serialize;
use surrealdb::types::{RecordIdKey, ToSql, Uuid};

use crate::{
    GlobalDBHandler,
    database::{
        boxes::Boxes,
        order::Order,
        product::{Product, ProductMarker},
    },
};

const MAX_ITEM_INSTANCES: usize = 10_000;
const MAX_FREE_SPACES_PER_BOX: usize = 4_096;
const WEAK_LAST_ATTEMPT: usize = 4;
const WEIGHT_INTERLEAVE_ATTEMPT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl Dimensions {
    fn volume(self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * u64::from(self.depth)
    }

    fn max_edge(self) -> u32 {
        self.width.max(self.height).max(self.depth)
    }

    fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0 || self.depth == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Position {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Orientation {
    #[serde(rename = "WHD")]
    Whd,
    #[serde(rename = "WDH")]
    Wdh,
    #[serde(rename = "HWD")]
    Hwd,
    #[serde(rename = "HDW")]
    Hdw,
    #[serde(rename = "DWH")]
    Dwh,
    #[serde(rename = "DHW")]
    Dhw,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackingStatus {
    Complete,
    Partial,
}

#[derive(Debug, Serialize)]
pub struct CoordinateSystem {
    origin: &'static str,
    x: &'static str,
    y: &'static str,
    z: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PackingSummary {
    products_requested: usize,
    products_placed: usize,
    boxes_used: usize,
    product_volume: u64,
    selected_box_volume: u64,
    volume_utilization: f64,
}

#[derive(Debug, Serialize)]
pub struct PackedBox {
    instance_id: String,
    box_type_id: String,
    size: Dimensions,
    max_weight: u64,
    total_weight: u64,
    used_volume: u64,
    volume_utilization: f64,
    placed_products: usize,
}

#[derive(Debug, Serialize)]
pub struct PackingStep {
    step: usize,
    action: &'static str,
    box_instance_id: String,
    product_instance_id: String,
    product_id: String,
    name: String,
    markers: Vec<ProductMarker>,
    position: Position,
    original_size: Dimensions,
    oriented_size: Dimensions,
    orientation: Orientation,
    supported_by: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UnplacedProduct {
    product_instance_id: String,
    product_id: String,
    name: String,
    reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PackingPlan {
    order_id: String,
    status: PackingStatus,
    algorithm: &'static str,
    units: &'static str,
    coordinate_system: CoordinateSystem,
    summary: PackingSummary,
    boxes: Vec<PackedBox>,
    steps: Vec<PackingStep>,
    unplaced_products: Vec<UnplacedProduct>,
}

#[derive(Debug)]
enum PackingError {
    MissingProduct(String),
    TooManyItems(usize),
    InvalidProductDimensions(String),
    InvalidBoxDimensions(String),
}

impl Display for PackingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProduct(id) => write!(formatter, "product {id} was not found"),
            Self::TooManyItems(count) => write!(
                formatter,
                "order contains {count} product instances; maximum is {MAX_ITEM_INSTANCES}"
            ),
            Self::InvalidProductDimensions(id) => {
                write!(formatter, "product {id} has a zero-sized dimension")
            }
            Self::InvalidBoxDimensions(id) => {
                write!(formatter, "box {id} has a zero-sized dimension")
            }
        }
    }
}

#[derive(Serialize)]
struct PackingErrorResponse {
    error: String,
}

#[post("/order/{id}/packing-plan")]
pub async fn create_packing_plan(
    id: web::Path<String>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let record_key = match Uuid::from_str(&id) {
        Ok(uuid) => RecordIdKey::Uuid(uuid),
        Err(error) => {
            return HttpResponse::BadRequest().json(PackingErrorResponse {
                error: format!("invalid order id: {error}"),
            });
        }
    };

    let db = db_handler.handler.clone();
    let order = match db.select::<Option<Order>>(("orders", record_key)).await {
        Ok(Some(order)) => order,
        Ok(None) => {
            return HttpResponse::NotFound().json(PackingErrorResponse {
                error: "order was not found".to_owned(),
            });
        }
        Err(error) => {
            return HttpResponse::InternalServerError().json(PackingErrorResponse {
                error: error.to_string(),
            });
        }
    };

    let product_ids: Vec<_> = order
        .get_products()
        .iter()
        .map(|(product_id, _)| product_id.clone())
        .collect();
    let products_query = db
        .query("SELECT * FROM products WHERE id IN $product_ids;")
        .bind(("product_ids", product_ids));
    let boxes_query = db.query("SELECT * FROM boxes WHERE count > 0;");
    let (products_result, boxes_result) = tokio::join!(products_query, boxes_query);
    let products = match products_result {
        Ok(mut response) => match response.take::<Vec<Product>>(0) {
            Ok(products) => products,
            Err(error) => {
                return HttpResponse::InternalServerError().json(PackingErrorResponse {
                    error: error.to_string(),
                });
            }
        },
        Err(error) => {
            return HttpResponse::InternalServerError().json(PackingErrorResponse {
                error: error.to_string(),
            });
        }
    };
    let boxes = match boxes_result {
        Ok(mut response) => match response.take::<Vec<Boxes>>(0) {
            Ok(boxes) => boxes,
            Err(error) => {
                return HttpResponse::InternalServerError().json(PackingErrorResponse {
                    error: error.to_string(),
                });
            }
        },
        Err(error) => {
            return HttpResponse::InternalServerError().json(PackingErrorResponse {
                error: error.to_string(),
            });
        }
    };

    match web::block(move || build_packing_plan(&order, &products, &boxes)).await {
        Ok(Ok(plan)) if matches!(plan.status, PackingStatus::Complete) => {
            HttpResponse::Ok().json(plan)
        }
        Ok(Ok(plan)) => HttpResponse::UnprocessableEntity().json(plan),
        Ok(Err(error)) => HttpResponse::UnprocessableEntity().json(PackingErrorResponse {
            error: error.to_string(),
        }),
        Err(error) => HttpResponse::InternalServerError().json(PackingErrorResponse {
            error: error.to_string(),
        }),
    }
}

#[derive(Clone, Debug)]
struct Item {
    product_instance_id: String,
    instance_index: usize,
    product_id: String,
    name: String,
    markers: Vec<ProductMarker>,
    size: Dimensions,
    weight: u64,
}

impl Item {
    fn volume(&self) -> u64 {
        self.size.volume()
    }

    fn is_weak(&self) -> bool {
        self.markers.contains(&ProductMarker::Weak)
    }
}

#[derive(Clone, Debug)]
struct StockBox {
    box_type_id: String,
    size: Dimensions,
    max_weight: u64,
    count: usize,
}

#[derive(Clone, Debug)]
struct PlacedItem {
    item: Item,
    position: Position,
    size: Dimensions,
    orientation: Orientation,
    supported_by: Vec<String>,
    sequence: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FreeSpace {
    position: Position,
    size: Dimensions,
}

impl FreeSpace {
    fn volume(self) -> u64 {
        self.size.volume()
    }

    fn max_x(self) -> u32 {
        self.position.x + self.size.width
    }

    fn max_y(self) -> u32 {
        self.position.y + self.size.height
    }

    fn max_z(self) -> u32 {
        self.position.z + self.size.depth
    }

    fn contains(self, other: Self) -> bool {
        self.position.x <= other.position.x
            && self.position.y <= other.position.y
            && self.position.z <= other.position.z
            && self.max_x() >= other.max_x()
            && self.max_y() >= other.max_y()
            && self.max_z() >= other.max_z()
    }

    fn intersects(self, other: Self) -> bool {
        self.position.x < other.max_x()
            && self.max_x() > other.position.x
            && self.position.y < other.max_y()
            && self.max_y() > other.position.y
            && self.position.z < other.max_z()
            && self.max_z() > other.position.z
    }

    fn split_around(self, occupied: Self) -> Vec<Self> {
        if !self.intersects(occupied) {
            return vec![self];
        }

        let x0 = self.position.x.max(occupied.position.x);
        let y0 = self.position.y.max(occupied.position.y);
        let z0 = self.position.z.max(occupied.position.z);
        let x1 = self.max_x().min(occupied.max_x());
        let y1 = self.max_y().min(occupied.max_y());
        let z1 = self.max_z().min(occupied.max_z());
        let mut result = Vec::with_capacity(6);

        push_space(
            &mut result,
            self.position,
            Dimensions {
                width: x0 - self.position.x,
                height: self.size.height,
                depth: self.size.depth,
            },
        );
        push_space(
            &mut result,
            Position {
                x: x1,
                ..self.position
            },
            Dimensions {
                width: self.max_x() - x1,
                height: self.size.height,
                depth: self.size.depth,
            },
        );
        push_space(
            &mut result,
            self.position,
            Dimensions {
                width: self.size.width,
                height: y0 - self.position.y,
                depth: self.size.depth,
            },
        );
        push_space(
            &mut result,
            Position {
                x: self.position.x,
                y: y1,
                z: self.position.z,
            },
            Dimensions {
                width: self.size.width,
                height: self.max_y() - y1,
                depth: self.size.depth,
            },
        );
        push_space(
            &mut result,
            self.position,
            Dimensions {
                width: self.size.width,
                height: self.size.height,
                depth: z0 - self.position.z,
            },
        );
        push_space(
            &mut result,
            Position {
                x: self.position.x,
                y: self.position.y,
                z: z1,
            },
            Dimensions {
                width: self.size.width,
                height: self.size.height,
                depth: self.max_z() - z1,
            },
        );

        result
    }
}

fn push_space(spaces: &mut Vec<FreeSpace>, position: Position, size: Dimensions) {
    if !size.is_empty() {
        spaces.push(FreeSpace { position, size });
    }
}

#[derive(Debug)]
struct OpenBox {
    instance_id: String,
    box_type_id: String,
    size: Dimensions,
    max_weight: u64,
    total_weight: u64,
    used_volume: u64,
    occupied_size: Dimensions,
    free_spaces: Vec<FreeSpace>,
    preferred_orientations: HashMap<String, Dimensions>,
    placements: Vec<PlacedItem>,
}

impl OpenBox {
    fn new(instance_id: String, stock: &StockBox) -> Self {
        Self {
            instance_id,
            box_type_id: stock.box_type_id.clone(),
            size: stock.size,
            max_weight: stock.max_weight,
            total_weight: 0,
            used_volume: 0,
            occupied_size: Dimensions {
                width: 0,
                height: 0,
                depth: 0,
            },
            free_spaces: vec![FreeSpace {
                position: Position { x: 0, y: 0, z: 0 },
                size: stock.size,
            }],
            preferred_orientations: HashMap::new(),
            placements: Vec::new(),
        }
    }

    fn place(&mut self, item: Item, candidate: PlacementCandidate, sequence: usize) {
        let occupied = FreeSpace {
            position: candidate.position,
            size: candidate.size,
        };
        let old_spaces = std::mem::take(&mut self.free_spaces);
        self.free_spaces = old_spaces
            .into_iter()
            .flat_map(|space| space.split_around(occupied))
            .collect();
        prune_free_spaces(&mut self.free_spaces);

        self.preferred_orientations
            .entry(item.product_id.clone())
            .or_insert(candidate.size);
        self.total_weight += item.weight;
        self.used_volume += item.volume();
        self.occupied_size.width = self
            .occupied_size
            .width
            .max(candidate.position.x + candidate.size.width);
        self.occupied_size.height = self
            .occupied_size
            .height
            .max(candidate.position.y + candidate.size.height);
        self.occupied_size.depth = self
            .occupied_size
            .depth
            .max(candidate.position.z + candidate.size.depth);
        self.placements.push(PlacedItem {
            item,
            position: candidate.position,
            size: candidate.size,
            orientation: candidate.orientation,
            supported_by: candidate.supported_by,
            sequence,
        });
    }
}

fn prune_free_spaces(spaces: &mut Vec<FreeSpace>) {
    spaces.sort_unstable_by(|left, right| {
        right
            .volume()
            .cmp(&left.volume())
            .then_with(|| left.position.y.cmp(&right.position.y))
            .then_with(|| left.position.z.cmp(&right.position.z))
            .then_with(|| left.position.x.cmp(&right.position.x))
            .then_with(|| left.size.cmp(&right.size))
    });
    spaces.dedup();

    let mut pruned: Vec<FreeSpace> = Vec::with_capacity(spaces.len().min(MAX_FREE_SPACES_PER_BOX));
    for space in spaces.drain(..) {
        if pruned.iter().any(|container| container.contains(space)) {
            continue;
        }
        pruned.push(space);
        if pruned.len() == MAX_FREE_SPACES_PER_BOX {
            break;
        }
    }
    *spaces = pruned;
}

#[derive(Debug)]
struct PlacementCandidate {
    position: Position,
    size: Dimensions,
    orientation: Orientation,
    supported_by: Vec<String>,
    free_space_waste: u64,
    contact_area: u64,
}

type PlacementScore = (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64);
type ExistingBoxScore = (u64, u64, u32, u64, u32, u32);
type BoxChoiceScore = (u8, u64, u64, u64, u64);

#[derive(Clone, Copy)]
enum ItemOrder {
    Volume,
    MaxEdge,
    Weight,
    WeightInterleave,
    MostConstrained,
    WeakLast,
}

#[derive(Clone, Copy)]
enum BoxPolicy {
    TargetRemaining,
    MaxGridCapacity,
    Largest,
    Smallest,
}

#[derive(Clone, Copy)]
enum PlacementPolicy {
    BestFit,
    GridCapacity,
}

#[derive(Debug)]
struct UnplacedItem {
    item: Item,
    reason: &'static str,
}

#[derive(Debug)]
struct PackingSolution {
    boxes: Vec<OpenBox>,
    unplaced: Vec<UnplacedItem>,
    requested: usize,
}

fn build_packing_plan(
    order: &Order,
    products: &[Product],
    boxes: &[Boxes],
) -> Result<PackingPlan, PackingError> {
    let items = build_items(order, products)?;
    let stock = build_stock(boxes)?;
    let order_id = order.get_id().to_sql();
    let solution = solve(items, stock);
    Ok(solution.into_plan(order_id))
}

fn build_items(order: &Order, products: &[Product]) -> Result<Vec<Item>, PackingError> {
    let products_by_id: HashMap<String, &Product> = products
        .iter()
        .map(|product| (product.get_id().to_sql(), product))
        .collect();
    let requested = order
        .get_products()
        .iter()
        .try_fold(0usize, |total, (_, count)| total.checked_add(*count))
        .unwrap_or(usize::MAX);
    if requested > MAX_ITEM_INSTANCES {
        return Err(PackingError::TooManyItems(requested));
    }

    let mut occurrences: HashMap<String, usize> = HashMap::new();
    let mut items = Vec::with_capacity(requested);
    for (product_id, count) in order.get_products() {
        if *count == 0 {
            continue;
        }
        let product_id = product_id.to_sql();
        let product = products_by_id
            .get(&product_id)
            .ok_or_else(|| PackingError::MissingProduct(product_id.clone()))?;
        let bounds = product.get_bounds();
        let size = Dimensions {
            width: u32::from(bounds.get_width()),
            height: u32::from(bounds.get_height()),
            depth: u32::from(bounds.get_depth()),
        };
        if size.is_empty() {
            return Err(PackingError::InvalidProductDimensions(product_id));
        }

        let occurrence = occurrences.entry(product_id.clone()).or_default();
        for _ in 0..*count {
            *occurrence += 1;
            items.push(Item {
                product_instance_id: format!("{product_id}#{occurrence}"),
                instance_index: *occurrence,
                product_id: product_id.clone(),
                name: product.get_name().to_owned(),
                markers: product.get_marker().unwrap_or_default().to_vec(),
                size,
                weight: u64::from(product.get_weight()),
            });
        }
    }

    Ok(items)
}

fn build_stock(boxes: &[Boxes]) -> Result<Vec<StockBox>, PackingError> {
    boxes
        .iter()
        .filter(|stock| stock.get_count() > 0)
        .map(|stock| {
            let bounds = stock.get_bounds();
            let box_type_id = stock.get_id().to_sql();
            let size = Dimensions {
                width: u32::from(bounds.get_width()),
                height: u32::from(bounds.get_height()),
                depth: u32::from(bounds.get_depth()),
            };
            if size.is_empty() {
                return Err(PackingError::InvalidBoxDimensions(box_type_id));
            }
            Ok(StockBox {
                box_type_id,
                size,
                max_weight: u64::from(stock.get_max_weight()),
                count: usize::from(stock.get_count()),
            })
        })
        .collect()
}

fn solve(items: Vec<Item>, mut stock: Vec<StockBox>) -> PackingSolution {
    stock.sort_unstable_by(|left, right| {
        left.size
            .volume()
            .cmp(&right.size.volume())
            .then_with(|| left.size.cmp(&right.size))
            .then_with(|| left.max_weight.cmp(&right.max_weight))
            .then_with(|| left.box_type_id.cmp(&right.box_type_id))
    });

    let attempts = [
        (
            ItemOrder::Volume,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::GridCapacity,
        ),
        (
            ItemOrder::Volume,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::BestFit,
        ),
        (
            ItemOrder::MostConstrained,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::GridCapacity,
        ),
        (
            ItemOrder::Volume,
            BoxPolicy::MaxGridCapacity,
            PlacementPolicy::GridCapacity,
        ),
        (
            ItemOrder::WeakLast,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::GridCapacity,
        ),
        (
            ItemOrder::WeightInterleave,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::BestFit,
        ),
        (
            ItemOrder::MaxEdge,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::BestFit,
        ),
        (
            ItemOrder::Weight,
            BoxPolicy::TargetRemaining,
            PlacementPolicy::BestFit,
        ),
        (
            ItemOrder::Volume,
            BoxPolicy::Largest,
            PlacementPolicy::BestFit,
        ),
        (
            ItemOrder::Volume,
            BoxPolicy::Smallest,
            PlacementPolicy::BestFit,
        ),
    ];
    let attempt_indices = selected_attempt_indices(&items, &stock, attempts.len());
    let mut best: Option<PackingSolution> = None;

    for attempt_index in attempt_indices {
        let (item_order, box_policy, placement_policy) = attempts[attempt_index];
        let candidate = pack_once(
            items.clone(),
            &stock,
            item_order,
            box_policy,
            placement_policy,
        );
        if best
            .as_ref()
            .is_none_or(|current| solution_is_better(&candidate, current))
        {
            best = Some(candidate);
        }
    }

    best.expect("at least one packing strategy is configured")
}

fn selected_attempt_indices(
    items: &[Item],
    stock: &[StockBox],
    attempt_count: usize,
) -> Vec<usize> {
    debug_assert!(attempt_count > WEIGHT_INTERLEAVE_ATTEMPT);
    if items.len() <= 1_000 {
        return (0..attempt_count).collect();
    }

    let homogeneous = items.first().is_none_or(|first| {
        items.iter().all(|item| {
            item.size == first.size
                && item.weight == first.weight
                && item.is_weak() == first.is_weak()
        })
    });
    let mut selected = if items.len() <= 5_000 {
        vec![0, 1, 2, 3]
    } else if homogeneous {
        vec![0]
    } else {
        vec![1]
    };

    let has_weak = items.iter().any(Item::is_weak);
    let has_strong = items.iter().any(|item| !item.is_weak());
    if has_weak && has_strong {
        selected.push(WEAK_LAST_ATTEMPT);
    }

    let mixed_weights = items
        .first()
        .is_some_and(|first| items.iter().any(|item| item.weight != first.weight));
    let total_weight = items
        .iter()
        .fold(0u64, |total, item| total.saturating_add(item.weight));
    let weight_can_constrain = stock
        .iter()
        .any(|stock_box| stock_box.count > 0 && stock_box.max_weight < total_weight);
    if mixed_weights && weight_can_constrain {
        selected.push(WEIGHT_INTERLEAVE_ATTEMPT);
    }

    selected
}

fn pack_once(
    mut items: Vec<Item>,
    stock: &[StockBox],
    item_order: ItemOrder,
    box_policy: BoxPolicy,
    placement_policy: PlacementPolicy,
) -> PackingSolution {
    sort_items(&mut items, item_order, stock);
    let requested = items.len();
    let mut stock_remaining: Vec<usize> = stock.iter().map(|item| item.count).collect();
    let mut open_boxes: Vec<OpenBox> = Vec::new();
    let mut unplaced = Vec::new();
    let mut next_sequence = 1;

    let mut suffix_volume = vec![0u64; items.len() + 1];
    let mut suffix_weight = vec![0u64; items.len() + 1];
    for index in (0..items.len()).rev() {
        suffix_volume[index] = suffix_volume[index + 1].saturating_add(items[index].volume());
        suffix_weight[index] = suffix_weight[index + 1].saturating_add(items[index].weight);
    }

    for (item_index, item) in items.into_iter().enumerate() {
        if let Some((box_index, candidate)) =
            find_existing_box(&open_boxes, &item, placement_policy)
        {
            open_boxes[box_index].place(item, candidate, next_sequence);
            next_sequence += 1;
            continue;
        }

        let selected_stock = choose_stock_box(
            stock,
            &stock_remaining,
            &item,
            suffix_volume[item_index],
            suffix_weight[item_index],
            box_policy,
            placement_policy,
        );
        if let Some((stock_index, candidate)) = selected_stock {
            stock_remaining[stock_index] -= 1;
            let mut new_box =
                OpenBox::new(format!("box-{}", open_boxes.len() + 1), &stock[stock_index]);
            new_box.place(item, candidate, next_sequence);
            next_sequence += 1;
            open_boxes.push(new_box);
        } else {
            unplaced.push(UnplacedItem {
                item,
                reason: "no_available_box_fits_dimensions_and_weight",
            });
        }
    }

    PackingSolution {
        boxes: open_boxes,
        unplaced,
        requested,
    }
}

fn sort_items(items: &mut [Item], item_order: ItemOrder, stock: &[StockBox]) {
    let compatibility_by_item = matches!(item_order, ItemOrder::MostConstrained).then(|| {
        items
            .iter()
            .map(|item| {
                (
                    (item.size, item.weight),
                    compatible_stock_count(item, stock),
                )
            })
            .collect::<HashMap<_, _>>()
    });

    items.sort_by(|left, right| {
        let primary = match item_order {
            ItemOrder::Volume => right.volume().cmp(&left.volume()),
            ItemOrder::MaxEdge => right.size.max_edge().cmp(&left.size.max_edge()),
            ItemOrder::Weight | ItemOrder::WeightInterleave => right.weight.cmp(&left.weight),
            ItemOrder::MostConstrained => {
                let compatibility = compatibility_by_item
                    .as_ref()
                    .expect("compatibility is calculated for constrained sorting");
                compatibility[&(left.size, left.weight)]
                    .cmp(&compatibility[&(right.size, right.weight)])
            }
            ItemOrder::WeakLast => left.is_weak().cmp(&right.is_weak()),
        };
        primary
            .then_with(|| right.volume().cmp(&left.volume()))
            .then_with(|| right.size.max_edge().cmp(&left.size.max_edge()))
            .then_with(|| right.weight.cmp(&left.weight))
            .then_with(|| left.is_weak().cmp(&right.is_weak()))
            .then_with(|| left.product_id.cmp(&right.product_id))
            .then_with(|| left.instance_index.cmp(&right.instance_index))
    });

    if matches!(item_order, ItemOrder::WeightInterleave) {
        let sorted = items.to_vec();
        let positive_weight_count = sorted.partition_point(|item| item.weight > 0);
        let mut high = 0;
        let mut low = positive_weight_count;
        for (index, item) in items[..positive_weight_count].iter_mut().enumerate() {
            let source = if index % 2 == 0 {
                let source = high;
                high += 1;
                source
            } else {
                low -= 1;
                low
            };
            *item = sorted[source].clone();
        }
        items[positive_weight_count..].clone_from_slice(&sorted[positive_weight_count..]);
    }
}

fn compatible_stock_count(item: &Item, stock: &[StockBox]) -> usize {
    stock
        .iter()
        .filter(|stock_box| {
            stock_box.count > 0
                && stock_box.max_weight >= item.weight
                && fits_with_rotation(item.size, stock_box.size)
        })
        .fold(0usize, |count, stock_box| {
            count.saturating_add(stock_box.count)
        })
}

fn fits_with_rotation(item: Dimensions, container: Dimensions) -> bool {
    let mut item_edges = [item.width, item.height, item.depth];
    let mut container_edges = [container.width, container.height, container.depth];
    item_edges.sort_unstable();
    container_edges.sort_unstable();
    item_edges
        .into_iter()
        .zip(container_edges)
        .all(|(item_edge, container_edge)| item_edge <= container_edge)
}

fn find_existing_box(
    boxes: &[OpenBox],
    item: &Item,
    placement_policy: PlacementPolicy,
) -> Option<(usize, PlacementCandidate)> {
    let mut best: Option<(usize, PlacementCandidate, ExistingBoxScore)> = None;

    for (box_index, open_box) in boxes.iter().enumerate() {
        let Some(candidate) = find_placement(open_box, item, placement_policy) else {
            continue;
        };
        let remaining_volume = open_box
            .size
            .volume()
            .saturating_sub(open_box.used_volume + item.volume());
        let score = (
            remaining_volume,
            candidate.free_space_waste,
            candidate.position.y,
            u64::MAX - candidate.contact_area,
            candidate.position.z,
            candidate.position.x,
        );
        if best
            .as_ref()
            .is_none_or(|(_, _, best_score)| score < *best_score)
        {
            best = Some((box_index, candidate, score));
        }
    }

    best.map(|(box_index, candidate, _)| (box_index, candidate))
}

fn choose_stock_box(
    stock: &[StockBox],
    stock_remaining: &[usize],
    item: &Item,
    remaining_volume: u64,
    remaining_weight: u64,
    policy: BoxPolicy,
    placement_policy: PlacementPolicy,
) -> Option<(usize, PlacementCandidate)> {
    let mut best: Option<(usize, PlacementCandidate, BoxChoiceScore)> = None;

    for (stock_index, stock_box) in stock.iter().enumerate() {
        if stock_remaining[stock_index] == 0 || stock_box.max_weight < item.weight {
            continue;
        }
        let empty_box = OpenBox::new(String::new(), stock_box);
        let Some(candidate) = find_placement(&empty_box, item, placement_policy) else {
            continue;
        };
        let box_volume = stock_box.size.volume();
        let grid_capacity_penalty = match placement_policy {
            PlacementPolicy::BestFit => 0,
            PlacementPolicy::GridCapacity => {
                u64::MAX - grid_capacity(stock_box.size, candidate.size, item.is_weak())
            }
        };
        let score = match policy {
            BoxPolicy::TargetRemaining
                if box_volume >= remaining_volume && stock_box.max_weight >= remaining_weight =>
            {
                (
                    0,
                    box_volume - remaining_volume,
                    stock_box.max_weight - remaining_weight,
                    grid_capacity_penalty,
                    candidate.free_space_waste,
                )
            }
            BoxPolicy::TargetRemaining => (
                1,
                u64::MAX - box_volume.min(remaining_volume),
                u64::MAX - stock_box.max_weight.min(remaining_weight),
                grid_capacity_penalty,
                candidate.free_space_waste,
            ),
            BoxPolicy::MaxGridCapacity => (
                0,
                grid_capacity_penalty,
                box_volume,
                stock_box.max_weight,
                candidate.free_space_waste,
            ),
            BoxPolicy::Largest => (
                0,
                u64::MAX - box_volume,
                u64::MAX - stock_box.max_weight,
                grid_capacity_penalty,
                candidate.free_space_waste,
            ),
            BoxPolicy::Smallest => (
                0,
                box_volume,
                stock_box.max_weight,
                grid_capacity_penalty,
                candidate.free_space_waste,
            ),
        };
        if best
            .as_ref()
            .is_none_or(|(_, _, best_score)| score < *best_score)
        {
            best = Some((stock_index, candidate, score));
        }
    }

    best.map(|(stock_index, candidate, _)| (stock_index, candidate))
}

fn find_placement(
    open_box: &OpenBox,
    item: &Item,
    policy: PlacementPolicy,
) -> Option<PlacementCandidate> {
    if open_box.total_weight + item.weight > open_box.max_weight {
        return None;
    }

    let mut best: Option<(PlacementCandidate, PlacementScore)> = None;
    for free_space in &open_box.free_spaces {
        for (orientation, size) in orientations(item.size) {
            if size.width > free_space.size.width
                || size.height > free_space.size.height
                || size.depth > free_space.size.depth
            {
                continue;
            }
            let position = free_space.position;
            if collides(position, size, &open_box.placements)
                || is_below_existing_product(position, size, &open_box.placements)
            {
                continue;
            }
            let Some(supported_by) = support_for(position, size, &open_box.placements) else {
                continue;
            };
            let contact_area = contact_area(position, size, open_box);
            let residual_edges = u64::from(free_space.size.width - size.width)
                + u64::from(free_space.size.height - size.height)
                + u64::from(free_space.size.depth - size.depth);
            let free_space_waste = free_space.volume() - size.volume();
            let orientation_change_penalty = match policy {
                PlacementPolicy::BestFit => 0,
                PlacementPolicy::GridCapacity => u8::from(
                    open_box
                        .preferred_orientations
                        .get(&item.product_id)
                        .is_some_and(|preferred| *preferred != size),
                ),
            };
            let grid_capacity_penalty = match policy {
                PlacementPolicy::BestFit => 0,
                PlacementPolicy::GridCapacity => {
                    u64::MAX - grid_capacity(open_box.size, size, item.is_weak())
                }
            };
            let prospective_occupied_size = Dimensions {
                width: open_box.occupied_size.width.max(position.x + size.width),
                height: open_box.occupied_size.height.max(position.y + size.height),
                depth: open_box.occupied_size.depth.max(position.z + size.depth),
            };
            let score = match policy {
                PlacementPolicy::BestFit => (
                    0,
                    0,
                    free_space_waste,
                    residual_edges,
                    u64::from(position.y),
                    u64::MAX - contact_area,
                    u64::from(position.z),
                    u64::from(position.x),
                    0,
                    0,
                ),
                PlacementPolicy::GridCapacity => (
                    u64::from(orientation_change_penalty),
                    grid_capacity_penalty,
                    u64::from(position.y),
                    u64::from(prospective_occupied_size.max_edge()),
                    prospective_occupied_size.volume(),
                    u64::from(position.z),
                    u64::from(position.x),
                    u64::MAX - contact_area,
                    free_space_waste,
                    residual_edges,
                ),
            };
            let candidate = PlacementCandidate {
                position,
                size,
                orientation,
                supported_by,
                free_space_waste,
                contact_area,
            };
            if best
                .as_ref()
                .is_none_or(|(_, best_score)| score < *best_score)
            {
                best = Some((candidate, score));
            }
        }
    }

    best.map(|(candidate, _)| candidate)
}

fn grid_capacity(container: Dimensions, item: Dimensions, weak: bool) -> u64 {
    let vertical_layers = if weak {
        u64::from(item.height <= container.height)
    } else {
        u64::from(container.height / item.height)
    };
    u64::from(container.width / item.width)
        * vertical_layers
        * u64::from(container.depth / item.depth)
}

fn orientations(original: Dimensions) -> Vec<(Orientation, Dimensions)> {
    let candidates = [
        (
            Orientation::Whd,
            (original.width, original.height, original.depth),
        ),
        (
            Orientation::Wdh,
            (original.width, original.depth, original.height),
        ),
        (
            Orientation::Hwd,
            (original.height, original.width, original.depth),
        ),
        (
            Orientation::Hdw,
            (original.height, original.depth, original.width),
        ),
        (
            Orientation::Dwh,
            (original.depth, original.width, original.height),
        ),
        (
            Orientation::Dhw,
            (original.depth, original.height, original.width),
        ),
    ];
    let mut result = Vec::with_capacity(6);
    for (orientation, (width, height, depth)) in candidates {
        let dimensions = Dimensions {
            width,
            height,
            depth,
        };
        if !result
            .iter()
            .any(|(_, existing_dimensions)| *existing_dimensions == dimensions)
        {
            result.push((orientation, dimensions));
        }
    }
    result
}

fn collides(position: Position, size: Dimensions, placements: &[PlacedItem]) -> bool {
    placements.iter().any(|placed| {
        position.x < placed.position.x + placed.size.width
            && position.x + size.width > placed.position.x
            && position.y < placed.position.y + placed.size.height
            && position.y + size.height > placed.position.y
            && position.z < placed.position.z + placed.size.depth
            && position.z + size.depth > placed.position.z
    })
}

fn is_below_existing_product(
    position: Position,
    size: Dimensions,
    placements: &[PlacedItem],
) -> bool {
    placements.iter().any(|placed| {
        position.y + size.height <= placed.position.y
            && overlap_length(
                position.x,
                position.x + size.width,
                placed.position.x,
                placed.position.x + placed.size.width,
            ) > 0
            && overlap_length(
                position.z,
                position.z + size.depth,
                placed.position.z,
                placed.position.z + placed.size.depth,
            ) > 0
    })
}

#[derive(Clone, Debug)]
struct SupportRectangle {
    x0: u32,
    x1: u32,
    z0: u32,
    z1: u32,
    product_instance_id: String,
}

fn support_for(
    position: Position,
    size: Dimensions,
    placements: &[PlacedItem],
) -> Option<Vec<String>> {
    if position.y == 0 {
        return Some(vec!["box-floor".to_owned()]);
    }

    for placed in placements {
        let placed_top = placed.position.y + placed.size.height;
        if placed_top <= position.y
            && placed.item.markers.contains(&ProductMarker::Weak)
            && overlap_length(
                position.x,
                position.x + size.width,
                placed.position.x,
                placed.position.x + placed.size.width,
            ) > 0
            && overlap_length(
                position.z,
                position.z + size.depth,
                placed.position.z,
                placed.position.z + placed.size.depth,
            ) > 0
        {
            return None;
        }
    }

    let mut supports = Vec::new();
    for placed in placements {
        if placed.position.y + placed.size.height != position.y {
            continue;
        }
        let x0 = position.x.max(placed.position.x);
        let x1 = (position.x + size.width).min(placed.position.x + placed.size.width);
        let z0 = position.z.max(placed.position.z);
        let z1 = (position.z + size.depth).min(placed.position.z + placed.size.depth);
        if x0 < x1 && z0 < z1 {
            supports.push(SupportRectangle {
                x0,
                x1,
                z0,
                z1,
                product_instance_id: placed.item.product_instance_id.clone(),
            });
        }
    }

    let required_area = u64::from(size.width) * u64::from(size.depth);
    if rectangle_union_area(&supports) < required_area {
        return None;
    }
    let mut supported_by: Vec<String> = supports
        .into_iter()
        .map(|support| support.product_instance_id)
        .collect();
    supported_by.sort_unstable();
    supported_by.dedup();
    Some(supported_by)
}

fn rectangle_union_area(rectangles: &[SupportRectangle]) -> u64 {
    let mut xs: Vec<u32> = rectangles
        .iter()
        .flat_map(|rectangle| [rectangle.x0, rectangle.x1])
        .collect();
    xs.sort_unstable();
    xs.dedup();
    let mut area = 0u64;

    for x_window in xs.windows(2) {
        let x0 = x_window[0];
        let x1 = x_window[1];
        let mut z_intervals: Vec<(u32, u32)> = rectangles
            .iter()
            .filter(|rectangle| rectangle.x0 <= x0 && rectangle.x1 >= x1)
            .map(|rectangle| (rectangle.z0, rectangle.z1))
            .collect();
        z_intervals.sort_unstable();
        let mut covered_z = 0u64;
        let mut current: Option<(u32, u32)> = None;
        for (z0, z1) in z_intervals {
            match current {
                Some((current_z0, current_z1)) if z0 <= current_z1 => {
                    current = Some((current_z0, current_z1.max(z1)));
                }
                Some((current_z0, current_z1)) => {
                    covered_z += u64::from(current_z1 - current_z0);
                    current = Some((z0, z1));
                }
                None => current = Some((z0, z1)),
            }
        }
        if let Some((z0, z1)) = current {
            covered_z += u64::from(z1 - z0);
        }
        area += u64::from(x1 - x0) * covered_z;
    }

    area
}

fn overlap_length(left_start: u32, left_end: u32, right_start: u32, right_end: u32) -> u32 {
    left_end
        .min(right_end)
        .saturating_sub(left_start.max(right_start))
}

fn contact_area(position: Position, size: Dimensions, open_box: &OpenBox) -> u64 {
    let mut area = 0u64;
    if position.x == 0 || position.x + size.width == open_box.size.width {
        area += u64::from(size.height) * u64::from(size.depth);
    }
    if position.y == 0 || position.y + size.height == open_box.size.height {
        area += u64::from(size.width) * u64::from(size.depth);
    }
    if position.z == 0 || position.z + size.depth == open_box.size.depth {
        area += u64::from(size.width) * u64::from(size.height);
    }

    for placed in &open_box.placements {
        if position.x + size.width == placed.position.x
            || placed.position.x + placed.size.width == position.x
        {
            area += u64::from(overlap_length(
                position.y,
                position.y + size.height,
                placed.position.y,
                placed.position.y + placed.size.height,
            )) * u64::from(overlap_length(
                position.z,
                position.z + size.depth,
                placed.position.z,
                placed.position.z + placed.size.depth,
            ));
        }
        if position.y + size.height == placed.position.y
            || placed.position.y + placed.size.height == position.y
        {
            area += u64::from(overlap_length(
                position.x,
                position.x + size.width,
                placed.position.x,
                placed.position.x + placed.size.width,
            )) * u64::from(overlap_length(
                position.z,
                position.z + size.depth,
                placed.position.z,
                placed.position.z + placed.size.depth,
            ));
        }
        if position.z + size.depth == placed.position.z
            || placed.position.z + placed.size.depth == position.z
        {
            area += u64::from(overlap_length(
                position.x,
                position.x + size.width,
                placed.position.x,
                placed.position.x + placed.size.width,
            )) * u64::from(overlap_length(
                position.y,
                position.y + size.height,
                placed.position.y,
                placed.position.y + placed.size.height,
            ));
        }
    }
    area
}

fn solution_is_better(candidate: &PackingSolution, current: &PackingSolution) -> bool {
    let candidate_complete = candidate.unplaced.is_empty();
    let current_complete = current.unplaced.is_empty();
    if candidate_complete != current_complete {
        return candidate_complete;
    }

    let candidate_placed = candidate.requested - candidate.unplaced.len();
    let current_placed = current.requested - current.unplaced.len();
    if candidate_placed != current_placed {
        return candidate_placed > current_placed;
    }

    let candidate_product_volume: u64 = candidate.boxes.iter().map(|item| item.used_volume).sum();
    let current_product_volume: u64 = current.boxes.iter().map(|item| item.used_volume).sum();
    if candidate_product_volume != current_product_volume {
        return candidate_product_volume > current_product_volume;
    }

    // With the same products placed, less selected box volume means better aggregate utilization.
    // The box count breaks ties instead of forcing a mostly empty oversized box to win.
    let candidate_box_volume: u64 = candidate.boxes.iter().map(|item| item.size.volume()).sum();
    let current_box_volume: u64 = current.boxes.iter().map(|item| item.size.volume()).sum();
    if candidate_box_volume != current_box_volume {
        return candidate_box_volume < current_box_volume;
    }
    if candidate.boxes.len() != current.boxes.len() {
        return candidate.boxes.len() < current.boxes.len();
    }

    minimum_utilization(&candidate.boxes)
        .partial_cmp(&minimum_utilization(&current.boxes))
        .unwrap_or(Ordering::Equal)
        == Ordering::Greater
}

fn minimum_utilization(boxes: &[OpenBox]) -> f64 {
    boxes
        .iter()
        .map(|item| item.used_volume as f64 / item.size.volume() as f64)
        .reduce(f64::min)
        .unwrap_or(1.0)
}

impl PackingSolution {
    fn into_plan(self, order_id: String) -> PackingPlan {
        let products_placed = self.requested - self.unplaced.len();
        let product_volume: u64 = self.boxes.iter().map(|item| item.used_volume).sum();
        let selected_box_volume: u64 = self.boxes.iter().map(|item| item.size.volume()).sum();
        let volume_utilization = if selected_box_volume == 0 {
            0.0
        } else {
            product_volume as f64 / selected_box_volume as f64
        };
        let mut steps = Vec::with_capacity(products_placed);
        let boxes: Vec<PackedBox> = self
            .boxes
            .into_iter()
            .map(|packed_box| {
                let box_volume = packed_box.size.volume();
                let box_utilization = packed_box.used_volume as f64 / box_volume as f64;
                for placement in &packed_box.placements {
                    steps.push(PackingStep {
                        step: placement.sequence,
                        action: "place",
                        box_instance_id: packed_box.instance_id.clone(),
                        product_instance_id: placement.item.product_instance_id.clone(),
                        product_id: placement.item.product_id.clone(),
                        name: placement.item.name.clone(),
                        markers: placement.item.markers.clone(),
                        position: placement.position,
                        original_size: placement.item.size,
                        oriented_size: placement.size,
                        orientation: placement.orientation,
                        supported_by: placement.supported_by.clone(),
                    });
                }
                PackedBox {
                    instance_id: packed_box.instance_id,
                    box_type_id: packed_box.box_type_id,
                    size: packed_box.size,
                    max_weight: packed_box.max_weight,
                    total_weight: packed_box.total_weight,
                    used_volume: packed_box.used_volume,
                    volume_utilization: box_utilization,
                    placed_products: packed_box.placements.len(),
                }
            })
            .collect();
        steps.sort_unstable_by_key(|step| step.step);

        PackingPlan {
            order_id,
            status: if self.unplaced.is_empty() {
                PackingStatus::Complete
            } else {
                PackingStatus::Partial
            },
            algorithm: "multi_start_maximal_spaces_v3",
            units: "model_units",
            coordinate_system: CoordinateSystem {
                origin: "bottom-left-back",
                x: "width",
                y: "height",
                z: "depth",
            },
            summary: PackingSummary {
                products_requested: self.requested,
                products_placed,
                boxes_used: boxes.len(),
                product_volume,
                selected_box_volume,
                volume_utilization,
            },
            boxes,
            steps,
            unplaced_products: self
                .unplaced
                .into_iter()
                .map(|unplaced| UnplacedProduct {
                    product_instance_id: unplaced.item.product_instance_id,
                    product_id: unplaced.item.product_id,
                    name: unplaced.item.name,
                    reason: unplaced.reason,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{App, http::StatusCode, test as actix_test, web};
    use serde_json::json;
    use surrealdb::{
        Surreal,
        engine::local::{Db, Mem},
        types::{RecordId, RecordIdKey, Uuid},
    };

    use super::*;
    use crate::database::{boxes::BoxBounds, order::OrderStatus, product::ProductBounds};

    fn item(id: &str, size: Dimensions, weight: u64) -> Item {
        Item {
            product_instance_id: format!("products:{id}#1"),
            instance_index: 1,
            product_id: format!("products:{id}"),
            name: id.to_owned(),
            markers: Vec::new(),
            size,
            weight,
        }
    }

    fn stock(id: &str, size: Dimensions, max_weight: u64, count: usize) -> StockBox {
        StockBox {
            box_type_id: format!("boxes:{id}"),
            size,
            max_weight,
            count,
        }
    }

    fn repeated_items(id: &str, size: Dimensions, weight: u64, count: usize) -> Vec<Item> {
        (1..=count)
            .map(|instance| {
                let mut item = item(id, size, weight);
                item.product_instance_id = format!("products:{id}#{instance}");
                item.instance_index = instance;
                item
            })
            .collect()
    }

    fn assert_solution_invariants(
        requested_items: &[Item],
        stock: &[StockBox],
        solution: &PackingSolution,
    ) {
        assert_eq!(solution.requested, requested_items.len());

        let mut expected_instances = HashMap::<String, usize>::new();
        for item in requested_items {
            *expected_instances
                .entry(item.product_instance_id.clone())
                .or_default() += 1;
        }
        assert!(
            expected_instances.values().all(|count| *count == 1),
            "test input must contain unique product instance ids"
        );

        let mut available_stock = HashMap::<String, usize>::new();
        for stock_box in stock {
            *available_stock
                .entry(stock_box.box_type_id.clone())
                .or_default() += stock_box.count;
        }

        let mut used_stock = HashMap::<String, usize>::new();
        let mut observed_instances = HashMap::<String, usize>::new();
        let mut sequences = Vec::new();
        for packed_box in &solution.boxes {
            assert!(
                stock.iter().any(|stock_box| {
                    stock_box.box_type_id == packed_box.box_type_id
                        && stock_box.size == packed_box.size
                        && stock_box.max_weight == packed_box.max_weight
                }),
                "packed box must originate from stock"
            );
            *used_stock
                .entry(packed_box.box_type_id.clone())
                .or_default() += 1;
            assert!(!packed_box.placements.is_empty());
            assert!(packed_box.total_weight <= packed_box.max_weight);
            assert_eq!(
                packed_box.total_weight,
                packed_box
                    .placements
                    .iter()
                    .map(|placement| placement.item.weight)
                    .sum::<u64>()
            );
            assert_eq!(
                packed_box.used_volume,
                packed_box
                    .placements
                    .iter()
                    .map(|placement| placement.item.volume())
                    .sum::<u64>()
            );
            let occupied_size = packed_box.placements.iter().fold(
                Dimensions {
                    width: 0,
                    height: 0,
                    depth: 0,
                },
                |occupied, placement| Dimensions {
                    width: occupied
                        .width
                        .max(placement.position.x + placement.size.width),
                    height: occupied
                        .height
                        .max(placement.position.y + placement.size.height),
                    depth: occupied
                        .depth
                        .max(placement.position.z + placement.size.depth),
                },
            );
            assert_eq!(packed_box.occupied_size, occupied_size);

            for (placement_index, placement) in packed_box.placements.iter().enumerate() {
                assert!(
                    u64::from(placement.position.x) + u64::from(placement.size.width)
                        <= u64::from(packed_box.size.width)
                );
                assert!(
                    u64::from(placement.position.y) + u64::from(placement.size.height)
                        <= u64::from(packed_box.size.height)
                );
                assert!(
                    u64::from(placement.position.z) + u64::from(placement.size.depth)
                        <= u64::from(packed_box.size.depth)
                );
                assert_eq!(placement.size.volume(), placement.item.volume());
                assert!(
                    orientations(placement.item.size)
                        .contains(&(placement.orientation, placement.size,))
                );

                let earlier = &packed_box.placements[..placement_index];
                assert!(!collides(placement.position, placement.size, earlier));
                assert!(!is_below_existing_product(
                    placement.position,
                    placement.size,
                    earlier,
                ));
                assert_eq!(
                    support_for(placement.position, placement.size, earlier),
                    Some(placement.supported_by.clone())
                );

                *observed_instances
                    .entry(placement.item.product_instance_id.clone())
                    .or_default() += 1;
                sequences.push(placement.sequence);
            }
        }

        for unplaced in &solution.unplaced {
            *observed_instances
                .entry(unplaced.item.product_instance_id.clone())
                .or_default() += 1;
        }
        assert_eq!(observed_instances, expected_instances);

        for (box_type_id, used) in used_stock {
            assert!(
                used <= available_stock
                    .get(&box_type_id)
                    .copied()
                    .unwrap_or_default(),
                "used more {box_type_id} boxes than available"
            );
        }

        sequences.sort_unstable();
        assert_eq!(sequences, (1..=sequences.len()).collect::<Vec<_>>());
    }

    fn next_random(seed: &mut u64, upper_bound: u32) -> u32 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*seed >> 32) as u32) % upper_bound
    }

    #[test]
    fn packs_adjacent_products_into_one_box() {
        let size = Dimensions {
            width: 5,
            height: 10,
            depth: 10,
        };
        let mut second = item("second", size, 1);
        second.product_instance_id = "products:second#1".to_owned();
        let solution = solve(
            vec![item("first", size, 1), second],
            vec![stock(
                "large",
                Dimensions {
                    width: 10,
                    height: 10,
                    depth: 10,
                },
                10,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].placements.len(), 2);
        assert!(!collides(
            solution.boxes[0].placements[1].position,
            solution.boxes[0].placements[1].size,
            &solution.boxes[0].placements[..1]
        ));
    }

    #[test]
    fn rotates_product_to_fit() {
        let solution = solve(
            vec![item(
                "rotated",
                Dimensions {
                    width: 2,
                    height: 3,
                    depth: 4,
                },
                1,
            )],
            vec![stock(
                "tight",
                Dimensions {
                    width: 3,
                    height: 2,
                    depth: 4,
                },
                1,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes[0].placements[0].size.width, 3);
        assert_eq!(solution.boxes[0].placements[0].size.height, 2);
        assert_eq!(
            solution.boxes[0].placements[0].orientation,
            Orientation::Hwd
        );
    }

    #[test]
    fn respects_box_weight_limit() {
        let size = Dimensions {
            width: 5,
            height: 5,
            depth: 5,
        };
        let solution = solve(
            vec![item("heavy-1", size, 6), item("heavy-2", size, 6)],
            vec![stock(
                "weight-limited",
                Dimensions {
                    width: 10,
                    height: 5,
                    depth: 5,
                },
                10,
                2,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 2);
        assert!(solution.boxes.iter().all(|item| item.total_weight <= 10));
    }

    #[test]
    fn prefers_one_large_box_over_two_small_boxes() {
        let item_size = Dimensions {
            width: 5,
            height: 10,
            depth: 10,
        };
        let solution = solve(
            vec![item("one", item_size, 1), item("two", item_size, 1)],
            vec![
                stock("small", item_size, 10, 2),
                stock(
                    "large",
                    Dimensions {
                        width: 10,
                        height: 10,
                        depth: 10,
                    },
                    10,
                    1,
                ),
            ],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].box_type_id, "boxes:large");
    }

    #[test]
    fn packs_twenty_thin_identical_items_in_one_medium_box() {
        for book in orientations(Dimensions {
            width: 21,
            height: 29,
            depth: 4,
        })
        .into_iter()
        .map(|(_, size)| size)
        {
            let items = repeated_items("book", book, 700, 20);
            let stock = vec![
                stock(
                    "medium",
                    Dimensions {
                        width: 50,
                        height: 40,
                        depth: 30,
                    },
                    15_000,
                    10,
                ),
                stock(
                    "large",
                    Dimensions {
                        width: 80,
                        height: 60,
                        depth: 50,
                    },
                    30_000,
                    5,
                ),
            ];
            let solution = solve(items.clone(), stock.clone());

            assert!(solution.unplaced.is_empty());
            assert_eq!(solution.boxes.len(), 1);
            assert_eq!(solution.boxes[0].box_type_id, "boxes:medium");
            assert_eq!(solution.boxes[0].placements.len(), 20);
            assert!(solution.boxes[0].placements.iter().all(|placement| {
                placement.size
                    == (Dimensions {
                        width: 21,
                        height: 4,
                        depth: 29,
                    })
            }));
            let mut x_positions = solution.boxes[0]
                .placements
                .iter()
                .map(|placement| placement.position.x)
                .collect::<Vec<_>>();
            x_positions.sort_unstable();
            x_positions.dedup();
            let mut y_positions = solution.boxes[0]
                .placements
                .iter()
                .map(|placement| placement.position.y)
                .collect::<Vec<_>>();
            y_positions.sort_unstable();
            y_positions.dedup();
            assert_eq!(x_positions, vec![0, 21]);
            assert_eq!(
                y_positions,
                (0..10).map(|layer| layer * 4).collect::<Vec<_>>()
            );
            assert!(
                solution.boxes[0]
                    .placements
                    .iter()
                    .all(|placement| placement.position.z == 0)
            );
            assert_solution_invariants(&items, &stock, &solution);
        }
    }

    #[test]
    fn packs_twenty_two_books_using_less_total_box_volume() {
        let book_size = Dimensions {
            width: 21,
            height: 29,
            depth: 4,
        };
        let items = repeated_items("book", book_size, 700, 22);
        let stock = vec![
            stock(
                "medium",
                Dimensions {
                    width: 50,
                    height: 40,
                    depth: 30,
                },
                15_000,
                10,
            ),
            stock(
                "large",
                Dimensions {
                    width: 80,
                    height: 60,
                    depth: 50,
                },
                30_000,
                5,
            ),
        ];

        let solution = solve(items.clone(), stock.clone());

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 2);
        assert!(
            solution
                .boxes
                .iter()
                .all(|packed_box| packed_box.box_type_id == "boxes:medium")
        );
        assert_eq!(
            solution
                .boxes
                .iter()
                .map(|packed_box| packed_box.size.volume())
                .sum::<u64>(),
            120_000
        );
        assert_eq!(
            solution
                .boxes
                .iter()
                .map(|packed_box| packed_box.total_weight)
                .sum::<u64>(),
            15_400
        );
        assert_solution_invariants(&items, &stock, &solution);
    }

    #[test]
    fn packs_twenty_five_books_into_two_medium_boxes_instead_of_one_large() {
        let book_size = Dimensions {
            width: 21,
            height: 29,
            depth: 4,
        };
        let items = repeated_items("book", book_size, 700, 25);
        let stock = vec![
            stock(
                "medium",
                Dimensions {
                    width: 50,
                    height: 40,
                    depth: 30,
                },
                15_000,
                10,
            ),
            stock(
                "large",
                Dimensions {
                    width: 80,
                    height: 60,
                    depth: 50,
                },
                30_000,
                5,
            ),
        ];

        let solution = solve(items.clone(), stock.clone());

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 2);
        assert!(
            solution
                .boxes
                .iter()
                .all(|packed_box| packed_box.box_type_id == "boxes:medium")
        );
        assert_eq!(
            solution
                .boxes
                .iter()
                .map(|packed_box| packed_box.size.volume())
                .sum::<u64>(),
            120_000
        );
        assert_eq!(
            solution
                .boxes
                .iter()
                .map(|packed_box| packed_box.placements.len())
                .sum::<usize>(),
            25
        );
        assert_solution_invariants(&items, &stock, &solution);
    }

    #[test]
    fn keeps_one_grid_orientation_when_capacities_tie() {
        let solution = solve(
            repeated_items(
                "thin",
                Dimensions {
                    width: 1,
                    height: 3,
                    depth: 5,
                },
                1,
                12,
            ),
            vec![stock(
                "grid",
                Dimensions {
                    width: 12,
                    height: 3,
                    depth: 6,
                },
                12,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].placements.len(), 12);
        assert!(
            solution.boxes[0]
                .placements
                .windows(2)
                .all(|items| items[0].size == items[1].size)
        );
    }

    #[test]
    fn chooses_box_shape_by_grid_capacity_independent_of_stock_order() {
        let cube = stock(
            "cube",
            Dimensions {
                width: 10,
                height: 10,
                depth: 10,
            },
            100,
            2,
        );
        let flat = stock(
            "flat",
            Dimensions {
                width: 20,
                height: 10,
                depth: 5,
            },
            100,
            1,
        );
        let item_size = Dimensions {
            width: 6,
            height: 6,
            depth: 4,
        };

        for stock_order in [
            vec![cube.clone(), flat.clone()],
            vec![flat.clone(), cube.clone()],
        ] {
            let solution = solve(repeated_items("shaped", item_size, 1, 3), stock_order);
            assert!(solution.unplaced.is_empty());
            assert_eq!(solution.boxes.len(), 1);
            assert_eq!(solution.boxes[0].box_type_id, "boxes:flat");
        }
    }

    #[test]
    fn equivalent_stock_is_selected_deterministically() {
        let box_size = Dimensions {
            width: 5,
            height: 5,
            depth: 5,
        };
        let alpha = stock("alpha", box_size, 10, 1);
        let zeta = stock("zeta", box_size, 10, 1);

        for stock_order in [
            vec![zeta.clone(), alpha.clone()],
            vec![alpha.clone(), zeta.clone()],
        ] {
            let solution = solve(vec![item("cube", box_size, 1)], stock_order);
            assert!(solution.unplaced.is_empty());
            assert_eq!(solution.boxes[0].box_type_id, "boxes:alpha");
        }
    }

    #[test]
    fn tries_intermediate_box_shape_instead_of_only_smallest_or_largest() {
        let boxes = vec![
            stock(
                "small-capacity",
                Dimensions {
                    width: 3,
                    height: 3,
                    depth: 4,
                },
                100,
                10,
            ),
            stock(
                "right-shape",
                Dimensions {
                    width: 4,
                    height: 4,
                    depth: 3,
                },
                100,
                1,
            ),
            stock(
                "large-capacity",
                Dimensions {
                    width: 3,
                    height: 3,
                    depth: 6,
                },
                100,
                10,
            ),
        ];
        let solution = solve(
            repeated_items(
                "cube",
                Dimensions {
                    width: 2,
                    height: 2,
                    depth: 2,
                },
                1,
                4,
            ),
            boxes,
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].box_type_id, "boxes:right-shape");
    }

    #[test]
    fn places_items_with_fewer_compatible_boxes_first() {
        let flexible = item(
            "a-flexible",
            Dimensions {
                width: 4,
                height: 4,
                depth: 2,
            },
            2,
        );
        let constrained = item(
            "b-constrained",
            Dimensions {
                width: 3,
                height: 3,
                depth: 3,
            },
            1,
        );
        let square = stock(
            "square",
            Dimensions {
                width: 4,
                height: 4,
                depth: 4,
            },
            10,
            1,
        );
        let flat = stock(
            "flat",
            Dimensions {
                width: 8,
                height: 4,
                depth: 2,
            },
            10,
            1,
        );

        for stock_order in [
            vec![square.clone(), flat.clone()],
            vec![flat.clone(), square.clone()],
        ] {
            let solution = solve(vec![flexible.clone(), constrained.clone()], stock_order);
            assert!(solution.unplaced.is_empty());
            assert_eq!(solution.boxes.len(), 2);
        }
    }

    #[test]
    fn interleaves_weights_to_avoid_a_greedy_dead_end() {
        let size = Dimensions {
            width: 1,
            height: 1,
            depth: 1,
        };
        let items = [6, 5, 3, 2, 2, 2]
            .into_iter()
            .enumerate()
            .map(|(index, weight)| item(&format!("weight-{index}"), size, weight))
            .collect();
        let solution = solve(
            items,
            vec![stock(
                "weight-bins",
                Dimensions {
                    width: 3,
                    height: 1,
                    depth: 1,
                },
                10,
                2,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 2);
        assert!(solution.boxes.iter().all(|item| item.total_weight == 10));
    }

    #[test]
    fn large_orders_keep_only_the_extra_strategies_their_constraints_need() {
        let unit = Dimensions {
            width: 1,
            height: 1,
            depth: 1,
        };
        let weak_stock = vec![stock(
            "weak-large-order",
            Dimensions {
                width: 1_000,
                height: 2,
                depth: 1,
            },
            1_001,
            1,
        )];

        let mut weak_order = repeated_items("strong", unit, 1, 1_000);
        let mut weak = item(
            "weak",
            Dimensions {
                width: 1_000,
                height: 1,
                depth: 1,
            },
            1,
        );
        weak.markers.push(ProductMarker::Weak);
        weak_order.push(weak);
        let weak_attempts = selected_attempt_indices(&weak_order, &weak_stock, 10);
        assert!(weak_attempts.contains(&WEAK_LAST_ATTEMPT));
        assert!(!weak_attempts.contains(&WEIGHT_INTERLEAVE_ATTEMPT));
        let weak_solution = solve(weak_order.clone(), weak_stock.clone());
        assert!(weak_solution.unplaced.is_empty());
        assert_eq!(weak_solution.boxes.len(), 1);
        assert_solution_invariants(&weak_order, &weak_stock, &weak_solution);

        let mut weighted_order = [6, 5, 3, 2, 2, 2]
            .into_iter()
            .enumerate()
            .map(|(index, weight)| item(&format!("weighted-{index}"), unit, weight))
            .collect::<Vec<_>>();
        weighted_order.extend(repeated_items("zero-weight", unit, 0, 995));
        let weight_stock = vec![stock(
            "weighted-large-order",
            Dimensions {
                width: 1_000,
                height: 1,
                depth: 1,
            },
            10,
            2,
        )];
        let weight_attempts = selected_attempt_indices(&weighted_order, &weight_stock, 10);
        assert!(weight_attempts.contains(&WEIGHT_INTERLEAVE_ATTEMPT));
        assert!(!weight_attempts.contains(&WEAK_LAST_ATTEMPT));
        let weight_solution = solve(weighted_order.clone(), weight_stock.clone());
        assert!(weight_solution.unplaced.is_empty());
        assert_eq!(weight_solution.boxes.len(), 2);
        assert_solution_invariants(&weighted_order, &weight_stock, &weight_solution);
    }

    #[test]
    fn packs_strong_items_before_weak_items_of_the_same_size() {
        let size = Dimensions {
            width: 5,
            height: 5,
            depth: 5,
        };
        let mut weak = item("a-weak", size, 1);
        weak.markers.push(ProductMarker::Weak);
        let strong = item("z-strong", size, 1);
        let solution = solve(
            vec![weak, strong],
            vec![stock(
                "vertical",
                Dimensions {
                    width: 5,
                    height: 10,
                    depth: 5,
                },
                10,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert!(!solution.boxes[0].placements[0].item.is_weak());
        assert!(solution.boxes[0].placements[1].item.is_weak());
    }

    #[test]
    fn weak_last_strategy_keeps_load_bearing_items_below_fragile_items() {
        let mut weak = item(
            "weak-slab",
            Dimensions {
                width: 4,
                height: 1,
                depth: 4,
            },
            5,
        );
        weak.markers.push(ProductMarker::Weak);
        let cube_size = Dimensions {
            width: 2,
            height: 2,
            depth: 2,
        };
        let mut items = repeated_items("strong-cube", cube_size, 1, 4);
        items.push(weak);
        let solution = solve(
            items,
            vec![stock(
                "exact",
                Dimensions {
                    width: 4,
                    height: 3,
                    depth: 4,
                },
                9,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert!(solution.boxes[0].placements.last().unwrap().item.is_weak());
        assert_eq!(solution.boxes[0].placements.last().unwrap().position.y, 2);
    }

    #[test]
    fn weak_items_use_floor_capacity_without_sharing_preferences_with_strong_items() {
        let size = Dimensions {
            width: 1,
            height: 1,
            depth: 2,
        };
        let strong = item("strong", size, 1);
        let mut weak_items = repeated_items("weak", size, 1, 3);
        for item in &mut weak_items {
            item.markers.push(ProductMarker::Weak);
        }
        let mut items = vec![strong];
        items.append(&mut weak_items);
        let solution = solve(
            items,
            vec![stock(
                "floor",
                Dimensions {
                    width: 2,
                    height: 2,
                    depth: 2,
                },
                10,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        let weak_count = solution.boxes[0]
            .placements
            .iter()
            .filter(|item| item.item.is_weak())
            .count();
        assert_eq!(weak_count, 3);
    }

    #[test]
    fn sorts_instance_numbers_numerically() {
        let mut items = repeated_items(
            "ordered",
            Dimensions {
                width: 1,
                height: 1,
                depth: 1,
            },
            1,
            12,
        );
        items.reverse();
        sort_items(&mut items, ItemOrder::Volume, &[]);

        assert_eq!(
            items
                .iter()
                .map(|item| item.instance_index)
                .collect::<Vec<_>>(),
            (1..=12).collect::<Vec<_>>()
        );
    }

    #[test]
    fn grid_capacity_strategy_handles_small_repeated_item_tilings() {
        for item_width in 1..=4 {
            for item_height in 1..=4 {
                for item_depth in 1..=4 {
                    let item_size = Dimensions {
                        width: item_width,
                        height: item_height,
                        depth: item_depth,
                    };
                    for box_width in 2..=6 {
                        for box_height in 2..=6 {
                            for box_depth in 2..=6 {
                                let box_size = Dimensions {
                                    width: box_width,
                                    height: box_height,
                                    depth: box_depth,
                                };
                                let capacity = orientations(item_size)
                                    .into_iter()
                                    .map(|(_, size)| {
                                        usize::try_from(
                                            u64::from(box_width / size.width)
                                                * u64::from(box_height / size.height)
                                                * u64::from(box_depth / size.depth),
                                        )
                                        .expect("small capacity should fit usize")
                                    })
                                    .max()
                                    .unwrap_or_default()
                                    .min(12);
                                if capacity < 2 {
                                    continue;
                                }

                                let stock = [stock("grid", box_size, u64::MAX, 1)];
                                let solution = pack_once(
                                    repeated_items("tile", item_size, 1, capacity),
                                    &stock,
                                    ItemOrder::Volume,
                                    BoxPolicy::TargetRemaining,
                                    PlacementPolicy::GridCapacity,
                                );
                                assert!(
                                    solution.unplaced.is_empty(),
                                    "failed to place {capacity} items {item_size:?} in {box_size:?}"
                                );
                                assert_eq!(solution.boxes.len(), 1);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reports_product_that_cannot_fit() {
        let solution = solve(
            vec![item(
                "oversized",
                Dimensions {
                    width: 11,
                    height: 10,
                    depth: 10,
                },
                1,
            )],
            vec![stock(
                "small",
                Dimensions {
                    width: 10,
                    height: 10,
                    depth: 10,
                },
                10,
                1,
            )],
        );

        assert_eq!(solution.unplaced.len(), 1);
        assert!(solution.boxes.is_empty());
    }

    #[test]
    fn respects_available_box_count() {
        let size = Dimensions {
            width: 10,
            height: 10,
            depth: 10,
        };
        let solution = solve(
            vec![item("first", size, 1), item("second", size, 1)],
            vec![stock("single", size, 10, 1)],
        );

        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].placements.len(), 1);
        assert_eq!(solution.unplaced.len(), 1);
    }

    #[test]
    fn empty_order_produces_complete_empty_plan() {
        let plan = solve(Vec::new(), Vec::new()).into_plan("orders:empty".to_owned());

        assert!(matches!(plan.status, PackingStatus::Complete));
        assert_eq!(plan.summary.products_requested, 0);
        assert_eq!(plan.summary.boxes_used, 0);
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn zero_quantity_does_not_require_a_product_record() {
        let missing_product = RecordId::new(
            "products",
            RecordIdKey::Uuid(Uuid::from_str("00000000-0000-0000-0000-000000000201").unwrap()),
        );
        let order = Order::from_data(
            RecordId::new(
                "orders",
                RecordIdKey::Uuid(Uuid::from_str("00000000-0000-0000-0000-000000000202").unwrap()),
            ),
            vec![(missing_product, 0)],
            Vec::new(),
            OrderStatus::NotPacked,
        );

        assert!(build_items(&order, &[]).unwrap().is_empty());
    }

    #[test]
    fn does_not_stack_on_weak_product() {
        let size = Dimensions {
            width: 5,
            height: 5,
            depth: 5,
        };
        let mut weak = item("weak", size, 1);
        weak.markers.push(ProductMarker::Weak);
        let stock = stock(
            "vertical",
            Dimensions {
                width: 5,
                height: 10,
                depth: 5,
            },
            10,
            1,
        );
        let mut open_box = OpenBox::new("box-1".to_owned(), &stock);
        let placement = find_placement(&open_box, &weak, PlacementPolicy::BestFit)
            .expect("weak product should fit");
        open_box.place(weak, placement, 1);

        assert!(
            find_placement(&open_box, &item("top", size, 1), PlacementPolicy::BestFit).is_none()
        );
    }

    #[test]
    fn can_place_product_across_multiple_supports() {
        let bottom_size = Dimensions {
            width: 5,
            height: 5,
            depth: 5,
        };
        let stock = stock(
            "bridge",
            Dimensions {
                width: 10,
                height: 10,
                depth: 5,
            },
            10,
            1,
        );
        let mut open_box = OpenBox::new("box-1".to_owned(), &stock);
        for (sequence, id) in ["left", "right"].into_iter().enumerate() {
            let product = item(id, bottom_size, 1);
            let placement = find_placement(&open_box, &product, PlacementPolicy::BestFit)
                .expect("bottom product should fit");
            open_box.place(product, placement, sequence + 1);
        }

        let bridge = item(
            "bridge",
            Dimensions {
                width: 10,
                height: 5,
                depth: 5,
            },
            1,
        );
        let placement = find_placement(&open_box, &bridge, PlacementPolicy::BestFit)
            .expect("bridge should fit");

        assert_eq!(placement.position.y, 5);
        assert_eq!(placement.supported_by.len(), 2);
    }

    #[test]
    fn prunes_duplicate_and_contained_free_spaces() {
        let container = FreeSpace {
            position: Position { x: 0, y: 0, z: 0 },
            size: Dimensions {
                width: 10,
                height: 10,
                depth: 10,
            },
        };
        let contained = FreeSpace {
            position: Position { x: 1, y: 1, z: 1 },
            size: Dimensions {
                width: 2,
                height: 2,
                depth: 2,
            },
        };
        let separate = FreeSpace {
            position: Position { x: 20, y: 0, z: 0 },
            size: Dimensions {
                width: 2,
                height: 2,
                depth: 2,
            },
        };
        let mut spaces = vec![contained, separate, container, container];

        prune_free_spaces(&mut spaces);

        assert_eq!(spaces, vec![container, separate]);
    }

    #[test]
    fn seeded_mixed_orders_preserve_all_solution_invariants() {
        let mut seed = 0x5eed_cafe_f00d_beefu64;

        for case_index in 0..48 {
            let product_type_count = 1 + next_random(&mut seed, 4) as usize;
            let product_specs = (0..product_type_count)
                .map(|_| {
                    (
                        Dimensions {
                            width: 1 + next_random(&mut seed, 6),
                            height: 1 + next_random(&mut seed, 6),
                            depth: 1 + next_random(&mut seed, 6),
                        },
                        u64::from(next_random(&mut seed, 10)),
                        next_random(&mut seed, 7) == 0,
                    )
                })
                .collect::<Vec<_>>();
            let mut occurrences = vec![0usize; product_type_count];
            let item_count = 1 + next_random(&mut seed, 12) as usize;
            let mut items = Vec::with_capacity(item_count);
            for _ in 0..item_count {
                let product_index = next_random(&mut seed, product_type_count as u32) as usize;
                let (size, weight, weak) = product_specs[product_index];
                occurrences[product_index] += 1;
                let mut product = item(
                    &format!("case-{case_index}-product-{product_index}"),
                    size,
                    weight,
                );
                product.instance_index = occurrences[product_index];
                product.product_instance_id =
                    format!("{}#{}", product.product_id, product.instance_index);
                if weak {
                    product.markers.push(ProductMarker::Weak);
                }
                items.push(product);
            }

            let stock_type_count = 1 + next_random(&mut seed, 3) as usize;
            let stock = (0..stock_type_count)
                .map(|stock_index| {
                    stock(
                        &format!("case-{case_index}-box-{stock_index}"),
                        Dimensions {
                            width: 2 + next_random(&mut seed, 6),
                            height: 2 + next_random(&mut seed, 6),
                            depth: 2 + next_random(&mut seed, 6),
                        },
                        u64::from(1 + next_random(&mut seed, 20)),
                        (1 + next_random(&mut seed, 3)) as usize,
                    )
                })
                .collect::<Vec<_>>();

            let solution = solve(items.clone(), stock.clone());
            assert_solution_invariants(&items, &stock, &solution);
        }
    }

    #[test]
    fn packs_hundreds_of_items() {
        let item_size = Dimensions {
            width: 1,
            height: 1,
            depth: 1,
        };
        let items = (0..216)
            .map(|index| item(&format!("cube-{index}"), item_size, 1))
            .collect();
        let solution = solve(
            items,
            vec![stock(
                "cube",
                Dimensions {
                    width: 6,
                    height: 6,
                    depth: 6,
                },
                216,
                1,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].placements.len(), 216);
    }

    #[test]
    fn packs_the_maximum_homogeneous_order_with_the_bounded_strategy_set() {
        let item_size = Dimensions {
            width: 1,
            height: 1,
            depth: 1,
        };
        let items = repeated_items("maximum", item_size, 1, MAX_ITEM_INSTANCES);
        let stock = vec![stock(
            "maximum-grid",
            Dimensions {
                width: 100,
                height: 1,
                depth: 100,
            },
            MAX_ITEM_INSTANCES as u64,
            1,
        )];

        let solution = solve(items.clone(), stock.clone());

        assert!(solution.unplaced.is_empty());
        assert_eq!(solution.boxes.len(), 1);
        assert_eq!(solution.boxes[0].placements.len(), MAX_ITEM_INSTANCES);
        assert_solution_invariants(&items, &stock, &solution);
    }

    #[test]
    fn packs_large_mixed_order() {
        let items = (0..500)
            .map(|index| {
                item(
                    &format!("mixed-{index}"),
                    Dimensions {
                        width: 1 + index % 5,
                        height: 1 + (index * 3) % 5,
                        depth: 1 + (index * 7) % 5,
                    },
                    1,
                )
            })
            .collect();
        let solution = solve(
            items,
            vec![stock(
                "mixed",
                Dimensions {
                    width: 20,
                    height: 20,
                    depth: 20,
                },
                10_000,
                20,
            )],
        );

        assert!(solution.unplaced.is_empty());
        assert_eq!(
            solution
                .boxes
                .iter()
                .map(|packed_box| packed_box.placements.len())
                .sum::<usize>(),
            500
        );
    }

    #[actix_web::test]
    async fn packing_plan_endpoint_returns_visualization_ready_json() {
        let db: Surreal<Db> = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("packing-test")
            .use_db("packing-test")
            .await
            .unwrap();
        db.query(include_str!("../migrations/create_boxes.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query(include_str!("../migrations/create_products.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query(include_str!("../migrations/create_orders.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();

        let product_uuid = Uuid::from_str("00000000-0000-0000-0000-000000000101").unwrap();
        let box_uuid = Uuid::from_str("00000000-0000-0000-0000-000000000102").unwrap();
        let order_uuid = Uuid::from_str("00000000-0000-0000-0000-000000000103").unwrap();
        let product_id = RecordId::new("products", RecordIdKey::Uuid(product_uuid));
        let box_id = RecordId::new("boxes", RecordIdKey::Uuid(box_uuid));
        let order_id = RecordId::new("orders", RecordIdKey::Uuid(order_uuid));
        let product_bounds: ProductBounds = serde_json::from_value(json!({
            "width": 5,
            "height": 5,
            "depth": 5
        }))
        .unwrap();
        let box_bounds: BoxBounds = serde_json::from_value(json!({
            "width": 10,
            "height": 5,
            "depth": 5
        }))
        .unwrap();

        db.create::<Option<Product>>("products")
            .content(Product::from_data(
                product_id.clone(),
                "test product".to_owned(),
                None,
                product_bounds,
                2,
            ))
            .await
            .unwrap()
            .expect("product should be created");
        db.create::<Option<Boxes>>("boxes")
            .content(Boxes::from_data(box_id, box_bounds, 10, 1))
            .await
            .unwrap()
            .expect("box should be created");
        db.create::<Option<Order>>("orders")
            .content(Order::from_data(
                order_id,
                vec![(product_id, 2)],
                Vec::new(),
                OrderStatus::NotPacked,
            ))
            .await
            .unwrap()
            .expect("order should be created");

        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(GlobalDBHandler {
                    handler: db.clone(),
                }))
                .service(create_packing_plan),
        )
        .await;
        let request = actix_test::TestRequest::post()
            .uri("/order/00000000-0000-0000-0000-000000000103/packing-plan")
            .to_request();
        let response = actix_test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["status"], "complete");
        assert_eq!(body["algorithm"], "multi_start_maximal_spaces_v3");
        assert_eq!(body["summary"]["boxes_used"], 1);
        assert_eq!(body["summary"]["products_placed"], 2);
        assert_eq!(body["steps"].as_array().unwrap().len(), 2);
        assert_eq!(body["coordinate_system"]["y"], "height");

        let invalid_id_request = actix_test::TestRequest::post()
            .uri("/order/not-a-uuid/packing-plan")
            .to_request();
        let invalid_id_response = actix_test::call_service(&app, invalid_id_request).await;
        assert_eq!(invalid_id_response.status(), StatusCode::BAD_REQUEST);

        let missing_order_request = actix_test::TestRequest::post()
            .uri("/order/00000000-0000-0000-0000-000000000999/packing-plan")
            .to_request();
        let missing_order_response = actix_test::call_service(&app, missing_order_request).await;
        assert_eq!(missing_order_response.status(), StatusCode::NOT_FOUND);

        db.delete::<Vec<Boxes>>("boxes").await.unwrap();
        let partial_request = actix_test::TestRequest::post()
            .uri("/order/00000000-0000-0000-0000-000000000103/packing-plan")
            .to_request();
        let partial_response = actix_test::call_service(&app, partial_request).await;
        assert_eq!(partial_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let partial_body: serde_json::Value = actix_test::read_body_json(partial_response).await;
        assert_eq!(partial_body["status"], "partial");
        assert_eq!(
            partial_body["unplaced_products"].as_array().unwrap().len(),
            2
        );
    }
}
