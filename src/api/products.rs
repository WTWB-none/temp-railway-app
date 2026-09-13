use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey};

use crate::{
    GlobalDBHandler,
    api::parse_uuid_record_id,
    database::product::{Product, ProductBounds, ProductMarker},
};

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
struct ProductCreateData {
    name: String,
    marker: Option<Vec<ProductMarker>>,
    bounds: ProductBounds,
    weight: u16,
    count: u32,
}

#[get("/product/{id}")]
pub async fn get_product_by_id(
    data: web::Data<GlobalDBHandler>,
    id: web::Path<String>,
) -> impl Responder {
    let db = data.handler.clone();
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("products", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let item = db.select::<Option<Product>>(&id).await.map_err(|e| {
        errors.push(e.to_string());
    });
    match item {
        Ok(item) => match item {
            Some(this) => HttpResponse::Ok().json(&this),
            None => HttpResponse::NotFound().body("product by id not found"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[post("/product")]
pub async fn create_new_products_with_data(
    data: web::Json<ProductCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    let id = RecordId::new("products", RecordIdKey::uuid());
    let new_product = Product::from_data(
        id,
        data.name.clone(),
        data.marker.clone(),
        data.bounds,
        data.weight,
        data.count,
    );
    let res = db
        .create::<Option<Product>>("products")
        .content(new_product)
        .await
        .map_err(|e| {
            errors.push(e.to_string());
        });
    match res {
        Ok(record) => {
            if let Some(data) = record {
                HttpResponse::Ok().json(&data)
            } else {
                HttpResponse::InternalServerError().body("not created product record in table")
            }
        }
        Err(_) => HttpResponse::InternalServerError().json(errors),
    }
}

#[get("/products")]
pub async fn get_all_products(db_handler: web::Data<GlobalDBHandler>) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    let records = db
        .select::<Vec<Product>>("products")
        .await
        .map_err(|e| errors.push(e.to_string()));
    match records {
        Ok(this) => HttpResponse::Ok().json(&this),
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[post("/product/update/{id}")]
pub async fn update_product_by_id(
    id: web::Path<String>,
    data: web::Json<ProductCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("products", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let result = db
        .update::<Option<Product>>(&id)
        .content(Product::from_data(
            id,
            data.name.clone(),
            data.marker.clone(),
            data.bounds,
            data.weight,
            data.count,
        ))
        .await
        .map_err(|e| errors.push(e.to_string()));
    match result {
        Ok(record) => match record {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found product with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[delete("/product/{id}")]
pub async fn delete_product_by_id(
    id: web::Path<String>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("products", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let deleted_product = db
        .delete::<Option<Product>>(id)
        .await
        .map_err(|e| errors.push(e.to_string()));
    match deleted_product {
        Ok(order) => match order {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found product with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}
