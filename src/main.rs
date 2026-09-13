use std::io::ErrorKind;

use actix_cors::Cors;
use actix_jwt_auth_middleware::{Authority, TokenSigner, use_jwt::UseJWTOnApp};
use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use ed25519_compact::KeyPair;
use surrealdb::{
    Surreal,
    engine::local::{Db, Mem},
};

use crate::{
    api::{
        algorithm::create_packing_plan,
        auth::{self, logout},
        boxes::{
            create_new_box_with_data, delete_box_by_id, get_all_boxes, get_box_by_id,
            update_box_by_id,
        },
        orders::{
            create_new_order_with_data, delete_order_by_id, get_all_orders, get_order_by_id,
            update_order_by_id,
        },
        products::{
            create_new_products_with_data, delete_product_by_id, get_all_products,
            get_product_by_id, update_product_by_id,
        },
        register,
    },
    database::user::User,
    guard::user_role_guard::UserRole,
};

use jwt_compact::alg::Ed25519;

mod api;
mod database;
mod fixtures;
mod guard;
mod traits;

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod test_support;

#[allow(dead_code)]
struct GlobalDBHandler {
    handler: Surreal<Db>,
}

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("hello")
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let db = Surreal::new::<Mem>(()).await.map_err(|_| {
        std::io::Error::new(
            ErrorKind::PermissionDenied,
            "Cannot connect to in-memory database",
        )
    })?;
    db.use_ns("service").await.map_err(|_| {
        std::io::Error::new(
            ErrorKind::PermissionDenied,
            "Cannot use 'service' namespace",
        )
    })?;
    db.use_db("service").await.map_err(|_| {
        std::io::Error::new(ErrorKind::PermissionDenied, "Cannot use 'service' database")
    })?;

    db.query(include_str!("migrations/create_users.surql"))
        .await
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .check()
        .unwrap();
    db.query(include_str!("migrations/create_jwt_sessions.surql"))
        .await
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .check()
        .unwrap();
    db.query(include_str!("migrations/create_boxes.surql"))
        .await
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .check()
        .unwrap();
    db.query(include_str!("migrations/create_products.surql"))
        .await
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .check()
        .unwrap();
    db.query(include_str!("migrations/create_orders.surql"))
        .await
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .check()
        .unwrap();

    for fixture in fixtures::DEVELOPMENT_FIXTURES {
        db.query(*fixture)
            .await
            .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e.to_string()))?
            .check()
            .unwrap();
    }

    let app_db = web::Data::new(GlobalDBHandler { handler: db });

    let key_pair = KeyPair::generate();
    let publick_key = key_pair.pk;
    let secret_key = key_pair.sk;
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".into())
        .trim()
        .parse()
        .unwrap();
    HttpServer::new(move || {
        let authority = Authority::<User, Ed25519, _, _>::new()
            .refresh_authorizer(|| async move { Ok(()) })
            .token_signer(Some(
                TokenSigner::new()
                    .signing_key(secret_key.clone())
                    .algorithm(Ed25519)
                    .build()
                    .unwrap(),
            ))
            .verifying_key(publick_key)
            .build()
            .unwrap();
        App::new()
            .wrap(Cors::permissive().allowed_origin("https://frame.s3-website.cloud.ru/"))
            .service(register::regster_user)
            .service(register::validate_email_field)
            .service(register::validate_password_field)
            .service(auth::authorize_user)
            .use_jwt(
                authority,
                web::scope("/api/v1")
                    .service(auth::restore_session)
                    .service(logout)
                    .service(create_packing_plan)
                    .service(get_box_by_id)
                    .service(get_all_boxes)
                    .service(get_all_products)
                    .service(get_product_by_id)
                    .service(get_all_orders)
                    .service(get_order_by_id)
                    .service(
                        web::scope("")
                            .guard(UserRole)
                            .service(create_new_box_with_data)
                            .service(create_new_order_with_data)
                            .service(create_new_products_with_data)
                            .service(update_box_by_id)
                            .service(update_order_by_id)
                            .service(update_product_by_id)
                            .service(delete_box_by_id)
                            .service(delete_order_by_id)
                            .service(delete_product_by_id),
                    ),
            )
            .app_data(app_db.clone())
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
