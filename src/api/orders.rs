use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey};

use crate::{
    GlobalDBHandler,
    api::parse_uuid_record_id,
    database::{
        boxes::Boxes,
        order::{Order, OrderStatus},
        product::Product,
    },
};

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
struct OrderCreateData {
    products: Vec<(RecordId, usize)>,
    package_box: Vec<(RecordId, usize)>,
    status: OrderStatus,
}

#[get("/order/{id}")]
pub async fn get_order_by_id(
    data: web::Data<GlobalDBHandler>,
    id: web::Path<String>,
) -> impl Responder {
    let db = data.handler.clone();
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("orders", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let item = db.select::<Option<Order>>(&id).await.map_err(|e| {
        errors.push(e.to_string());
    });
    match item {
        Ok(item) => match item {
            Some(this) => HttpResponse::Ok().json(&this),
            None => HttpResponse::NotFound().body("order by id not found"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[post("/order")]
pub async fn create_new_order_with_data(
    data: web::Json<OrderCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    for (product, count) in data.products.clone() {
        let prod = db
            .select::<Option<Product>>(product)
            .await
            .map_err(|e| errors.push(e.to_string()))
            .unwrap();
        match prod {
            Some(this) => {
                if (this.get_count() as usize) < count {
                    errors.push(format!("not enough {} on stock", this.get_name()));
                }
            }
            None => {
                errors.push("not found product with such id".to_string());
            }
        }
    }
    if errors.is_empty() {
        let id = RecordId::new("orders", RecordIdKey::uuid());
        let new_order = Order::from_data(
            id,
            data.products.clone(),
            data.package_box.clone(),
            data.status.clone(),
        );
        let res = db
            .create::<Option<Order>>("orders")
            .content(new_order)
            .await
            .map_err(|e| {
                errors.push(e.to_string());
            });
        match res {
            Ok(record) => {
                if let Some(data) = record {
                    HttpResponse::Ok().json(&data)
                } else {
                    HttpResponse::InternalServerError().body("not created order record in table")
                }
            }
            Err(_) => HttpResponse::InternalServerError().json(errors),
        }
    } else {
        HttpResponse::BadRequest().json(&errors)
    }
}

#[get("/orders")]
pub async fn get_all_orders(db_handler: web::Data<GlobalDBHandler>) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    let records = db
        .select::<Vec<Order>>("orders")
        .await
        .map_err(|e| errors.push(e.to_string()));
    match records {
        Ok(this) => HttpResponse::Ok().json(&this),
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[post("/order/update/{id}")]
pub async fn update_order_by_id(
    id: web::Path<String>,
    data: web::Json<OrderCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("orders", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let result = db
        .update::<Option<Order>>(&id)
        .content(Order::from_data(
            id,
            data.products.clone(),
            data.package_box.clone(),
            data.status.clone(),
        ))
        .await
        .map_err(|e| errors.push(e.to_string()));
    match result {
        Ok(record) => match record {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found order with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[delete("/order/{id}")]
pub async fn delete_order_by_id(
    id: web::Path<String>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("orders", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let deleted_order = db
        .delete::<Option<Order>>(id)
        .await
        .map_err(|e| errors.push(e.to_string()));
    match deleted_order {
        Ok(order) => match order {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found order with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}
