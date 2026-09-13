use actix_web::web;
use surrealdb::{
    Surreal,
    engine::local::{Db, Mem},
    types::{RecordId, RecordIdKey},
};

use crate::GlobalDBHandler;

pub async fn database() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(())
        .await
        .expect("in-memory database should start");
    db.use_ns("backend-tests")
        .use_db("backend-tests")
        .await
        .expect("test namespace and database should be selected");

    for migration in [
        include_str!("migrations/create_users.surql"),
        include_str!("migrations/create_jwt_sessions.surql"),
        include_str!("migrations/create_boxes.surql"),
        include_str!("migrations/create_products.surql"),
        include_str!("migrations/create_orders.surql"),
    ] {
        db.query(migration)
            .await
            .expect("migration query should execute")
            .check()
            .expect("migration should be valid");
    }

    db
}

pub fn database_data(db: &Surreal<Db>) -> web::Data<GlobalDBHandler> {
    web::Data::new(GlobalDBHandler {
        handler: db.clone(),
    })
}

pub fn uuid_path(id: &RecordId) -> String {
    match &id.key {
        RecordIdKey::Uuid(uuid) => uuid.to_string(),
        other => panic!("expected UUID record key, got {other:?}"),
    }
}

#[tokio::test]
async fn development_fixtures_are_idempotent_and_consistent() {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    use crate::{
        api::parse_uuid_record_id,
        database::{
            boxes::Boxes,
            order::{Order, OrderStatus},
            product::{Product, ProductMarker},
            registration_type::RegistrationType,
            user::UserRoles,
        },
        fixtures::DEVELOPMENT_FIXTURES,
    };

    let db = database().await;
    for _ in 0..2 {
        for fixture in DEVELOPMENT_FIXTURES {
            db.query(*fixture)
                .await
                .expect("fixture query should execute")
                .check()
                .expect("fixture should be valid");
        }
    }

    let mut result = db
        .query("SELECT * FROM users WHERE email = $email;")
        .bind(("email", "superadmin@gmail.com"))
        .await
        .expect("fixture user should be queryable");
    let users = result
        .take::<Vec<crate::database::user::User>>(0)
        .expect("fixture user should deserialize");

    assert_eq!(users.len(), 1);
    let user = &users[0];
    assert_eq!(user.get_role(), &UserRoles::SuperAdmin);
    assert!(user.get_registration_type() == &RegistrationType::Default);
    let hash = PasswordHash::new(
        user.get_hash()
            .as_deref()
            .expect("password hash is required"),
    )
    .expect("fixture password hash should be valid");
    assert!(
        Argon2::default()
            .verify_password(b"Admin123!", &hash)
            .is_ok()
    );

    let products = db
        .select::<Vec<Product>>("products")
        .await
        .expect("fixture products should deserialize");
    let boxes = db
        .select::<Vec<Boxes>>("boxes")
        .await
        .expect("fixture boxes should deserialize");
    let orders = db
        .select::<Vec<Order>>("orders")
        .await
        .expect("fixture orders should deserialize");
    assert_eq!(products.len(), 4);
    assert_eq!(boxes.len(), 3);
    assert_eq!(orders.len(), 2);

    let vase_id = parse_uuid_record_id("products", "018f1000-0000-7000-8000-000000001001")
        .expect("fixture product ID should be valid");
    let vase = products
        .iter()
        .find(|product| product.get_id() == &vase_id)
        .expect("fixture vase should exist");
    assert_eq!(vase.get_name(), "Стеклянная ваза");
    assert_eq!(vase.get_marker(), Some([ProductMarker::Weak].as_slice()));
    assert_eq!(vase.get_count(), 20);

    let medium_box_id = parse_uuid_record_id("boxes", "018f1000-0000-7000-8000-000000002002")
        .expect("fixture box ID should be valid");
    let medium_box = boxes
        .iter()
        .find(|item| item.get_id() == &medium_box_id)
        .expect("fixture medium box should exist");
    assert_eq!(medium_box.get_count(), 10);

    let packed_order_id = parse_uuid_record_id("orders", "018f1000-0000-7000-8000-000000003002")
        .expect("fixture order ID should be valid");
    let packed_order = orders
        .iter()
        .find(|order| order.get_id() == &packed_order_id)
        .expect("fixture packed order should exist");
    assert_eq!(packed_order.get_status(), &OrderStatus::Packed);
    assert_eq!(
        packed_order.get_package_box(),
        [(medium_box_id, 1)].as_slice()
    );
}
