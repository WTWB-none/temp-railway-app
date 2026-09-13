use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey};

use crate::{
    GlobalDBHandler,
    api::parse_uuid_record_id,
    database::boxes::{BoxBounds, Boxes},
};

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
struct BoxCreateData {
    bounds: BoxBounds,
    max_weight: u16,
    count: u8,
}

#[get("/box/{id}")]
pub async fn get_box_by_id(
    data: web::Data<GlobalDBHandler>,
    id: web::Path<String>,
) -> impl Responder {
    let db = data.handler.clone();
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("boxes", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let item = db.select::<Option<Boxes>>(&id).await.map_err(|e| {
        errors.push(e.to_string());
    });
    match item {
        Ok(item) => match item {
            Some(this) => HttpResponse::Ok().json(&this),
            None => HttpResponse::NoContent().body("box by id not found"),
        },
        Err(_) => HttpResponse::NoContent().json(&errors),
    }
}

#[post("/box")]
pub async fn create_new_box_with_data(
    data: web::Json<BoxCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    let records: Option<Boxes> = db
        .query("SELECT * FROM boxes WHERE bounds = $bounds AND max_weight = $weight;")
        .bind(("bounds", data.bounds))
        .bind(("weight", data.max_weight))
        .await
        .unwrap()
        .take(0)
        .unwrap();
    if let Some(record) = records {
        let updated_box = record.clone();
        let Some(update) = updated_box.increase_count(data.count) else {
            return HttpResponse::BadRequest().body("box count exceeds 255");
        };
        let res = db
            .update::<Option<Boxes>>(updated_box.get_id())
            .content(update)
            .await
            .map_err(|e| errors.push(e.to_string()));
        match res {
            Ok(record) => match record {
                Some(data) => HttpResponse::Ok().json(&data),
                None => HttpResponse::InternalServerError().body("wtf was that???"),
            },
            Err(_) => HttpResponse::InternalServerError().json(&errors),
        }
    } else {
        let id = RecordId::new("boxes", RecordIdKey::uuid());
        let new_box = Boxes::from_data(id, data.bounds, data.max_weight, data.count);
        let res = db
            .create::<Option<Boxes>>("boxes")
            .content(new_box)
            .await
            .map_err(|e| {
                errors.push(e.to_string());
            });
        match res {
            Ok(record) => {
                if let Some(data) = record {
                    HttpResponse::Ok().json(&data)
                } else {
                    HttpResponse::NotAcceptable().body("not created box record in table")
                }
            }
            Err(_) => HttpResponse::InternalServerError().json(errors),
        }
    }
}

#[get("/boxes")]
pub async fn get_all_boxes(db_handler: web::Data<GlobalDBHandler>) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let db = db_handler.handler.clone();
    let records = db
        .select::<Vec<Boxes>>("boxes")
        .await
        .map_err(|e| errors.push(e.to_string()));
    match records {
        Ok(this) => HttpResponse::Ok().json(&this),
        Err(_) => HttpResponse::NotFound().json(&errors),
    }
}

#[post("/box/update/{id}")]
pub async fn update_box_by_id(
    id: web::Path<String>,
    data: web::Json<BoxCreateData>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("boxes", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let result = db
        .update::<Option<Boxes>>(&id)
        .content(Boxes::from_data(
            id,
            data.bounds,
            data.max_weight,
            data.count,
        ))
        .await
        .map_err(|e| errors.push(e.to_string()));
    match result {
        Ok(record) => match record {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found box with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}

#[delete("/box/{id}")]
pub async fn delete_box_by_id(
    id: web::Path<String>,
    db_handler: web::Data<GlobalDBHandler>,
) -> impl Responder {
    let mut errors: Vec<String> = vec![];
    let id = match parse_uuid_record_id("boxes", &id) {
        Ok(id) => id,
        Err(error) => return HttpResponse::BadRequest().body(error),
    };
    let db = db_handler.handler.clone();
    let deleted_order = db
        .delete::<Option<Boxes>>(id)
        .await
        .map_err(|e| errors.push(e.to_string()));
    match deleted_order {
        Ok(order) => match order {
            Some(data) => HttpResponse::Ok().json(&data),
            None => HttpResponse::NotFound().body("not found box with this id"),
        },
        Err(_) => HttpResponse::InternalServerError().json(&errors),
    }
}
