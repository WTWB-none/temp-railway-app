use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};

#[derive(Deserialize, Serialize, SurrealValue, Clone, PartialEq, Debug)]
pub struct Product {
    id: RecordId,
    name: String,
    marker: Option<Vec<ProductMarker>>,
    bounds: ProductBounds,
    weight: u16,
    count: u32,
    volume: u64,
}

#[derive(Serialize, Deserialize, SurrealValue, Clone, PartialEq, Debug)]
pub enum ProductMarker {
    Weak,
    Water,
    Fire,
}

#[derive(Serialize, Deserialize, SurrealValue, Clone, PartialEq, Debug, Copy)]
pub struct ProductBounds {
    width: u16,
    height: u16,
    depth: u16,
}

impl Product {
    pub fn from_data(
        id: RecordId,
        name: String,
        marker: Option<Vec<ProductMarker>>,
        bounds: ProductBounds,
        weight: u16,
        count: u32,
    ) -> Self {
        Self {
            id,
            name,
            marker,
            bounds,
            weight,
            count,
            volume: bounds.get_volume(),
        }
    }

    pub fn get_id(&self) -> &RecordId {
        &self.id
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_marker(&self) -> Option<&[ProductMarker]> {
        self.marker.as_deref()
    }

    pub fn get_bounds(&self) -> &ProductBounds {
        &self.bounds
    }

    pub fn get_weight(&self) -> u16 {
        self.weight
    }

    pub fn get_count(&self) -> u32 {
        self.count
    }
}

impl ProductBounds {
    pub(self) fn get_volume(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * u64::from(self.depth)
    }

    pub fn get_width(&self) -> u16 {
        self.width
    }

    pub fn get_height(&self) -> u16 {
        self.height
    }

    pub fn get_depth(&self) -> u16 {
        self.depth
    }
}
