use actix_jwt_auth_middleware::{Authority, TokenSigner, use_jwt::UseJWTOnApp};
use actix_web::{App, http::StatusCode, test as actix_test, web};
use ed25519_compact::KeyPair;
use jwt_compact::alg::Ed25519;
use serde::Serialize;
use serde_json::json;
use surrealdb::types::{RecordId, RecordIdKey};

use crate::{
    api::{
        auth::{authorize_user, logout, restore_session},
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
        register::{regster_user, validate_email_field, validate_password_field},
    },
    database::{
        boxes::Boxes,
        order::{Order, OrderStatus},
        product::{Product, ProductMarker},
        user::{User, UserRoles},
    },
    guard::user_role_guard::UserRole,
    test_support::{database, database_data, uuid_path},
};

#[actix_web::test]
async fn products_crud_and_invalid_id_are_handled_over_http() {
    let db = database().await;
    let app = actix_test::init_service(
        App::new().app_data(database_data(&db)).service(
            web::scope("/api/v1")
                .service(create_new_products_with_data)
                .service(get_all_products)
                .service(get_product_by_id)
                .service(update_product_by_id)
                .service(delete_product_by_id),
        ),
    )
    .await;

    let create_request = actix_test::TestRequest::post()
        .uri("/api/v1/product")
        .set_json(json!({
            "name": "glass",
            "marker": ["Weak", "Water"],
            "bounds": { "width": 20, "height": 30, "depth": 40 },
            "weight": 5,
            "count": 25
        }))
        .to_request();
    let created: Product = actix_test::call_and_read_body_json(&app, create_request).await;
    assert_eq!(created.get_name(), "glass");
    assert_eq!(
        created.get_marker(),
        Some([ProductMarker::Weak, ProductMarker::Water].as_slice())
    );
    assert_eq!(created.get_bounds().get_width(), 20);
    assert_eq!(created.get_count(), 25);
    let product_path = uuid_path(created.get_id());

    let get_request = actix_test::TestRequest::get()
        .uri(&format!("/api/v1/product/{product_path}"))
        .to_request();
    let fetched: Product = actix_test::call_and_read_body_json(&app, get_request).await;
    assert_eq!(fetched, created);

    let update_request = actix_test::TestRequest::post()
        .uri(&format!("/api/v1/product/update/{product_path}"))
        .set_json(json!({
            "name": "reinforced glass",
            "marker": ["Water"],
            "bounds": { "width": 20, "height": 30, "depth": 40 },
            "weight": 6,
            "count": 12
        }))
        .to_request();
    let updated: Product = actix_test::call_and_read_body_json(&app, update_request).await;
    assert_eq!(updated.get_name(), "reinforced glass");
    assert_eq!(updated.get_weight(), 6);
    assert_eq!(updated.get_count(), 12);

    let list_request = actix_test::TestRequest::get()
        .uri("/api/v1/products")
        .to_request();
    let products: Vec<Product> = actix_test::call_and_read_body_json(&app, list_request).await;
    assert_eq!(products, vec![updated.clone()]);

    let invalid_id_request = actix_test::TestRequest::get()
        .uri("/api/v1/product/not-a-uuid")
        .to_request();
    let invalid_id_response = actix_test::call_service(&app, invalid_id_request).await;
    assert_eq!(invalid_id_response.status(), StatusCode::BAD_REQUEST);

    let delete_request = actix_test::TestRequest::delete()
        .uri(&format!("/api/v1/product/{product_path}"))
        .to_request();
    let deleted: Product = actix_test::call_and_read_body_json(&app, delete_request).await;
    assert_eq!(deleted, updated);

    let missing_request = actix_test::TestRequest::get()
        .uri(&format!("/api/v1/product/{product_path}"))
        .to_request();
    let missing_response = actix_test::call_service(&app, missing_request).await;
    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn boxes_crud_merges_equal_stock_records() {
    let db = database().await;
    let app = actix_test::init_service(
        App::new().app_data(database_data(&db)).service(
            web::scope("/api/v1")
                .service(create_new_box_with_data)
                .service(get_all_boxes)
                .service(get_box_by_id)
                .service(update_box_by_id)
                .service(delete_box_by_id),
        ),
    )
    .await;
    let payload = json!({
        "bounds": { "width": 100, "height": 80, "depth": 60 },
        "max_weight": 500,
        "count": 2
    });

    let first_request = actix_test::TestRequest::post()
        .uri("/api/v1/box")
        .set_json(&payload)
        .to_request();
    let first: Boxes = actix_test::call_and_read_body_json(&app, first_request).await;
    assert_eq!(first.get_count(), 2);

    let second_request = actix_test::TestRequest::post()
        .uri("/api/v1/box")
        .set_json(&payload)
        .to_request();
    let merged: Boxes = actix_test::call_and_read_body_json(&app, second_request).await;
    assert_eq!(merged.get_id(), first.get_id());
    assert_eq!(merged.get_count(), 4);
    let box_path = uuid_path(merged.get_id());

    let list_request = actix_test::TestRequest::get()
        .uri("/api/v1/boxes")
        .to_request();
    let boxes: Vec<Boxes> = actix_test::call_and_read_body_json(&app, list_request).await;
    assert_eq!(boxes.len(), 1);

    let update_request = actix_test::TestRequest::post()
        .uri(&format!("/api/v1/box/update/{box_path}"))
        .set_json(json!({
            "bounds": { "width": 120, "height": 90, "depth": 70 },
            "max_weight": 600,
            "count": 7
        }))
        .to_request();
    let updated: Boxes = actix_test::call_and_read_body_json(&app, update_request).await;
    assert_eq!(updated.get_count(), 7);
    assert_eq!(updated.get_max_weight(), 600);
    assert_eq!(updated.get_bounds().get_width(), 120);

    let invalid_id_request = actix_test::TestRequest::get()
        .uri("/api/v1/box/not-a-uuid")
        .to_request();
    let invalid_id_response = actix_test::call_service(&app, invalid_id_request).await;
    assert_eq!(invalid_id_response.status(), StatusCode::BAD_REQUEST);

    let large_stock_payload = json!({
        "bounds": { "width": 10, "height": 10, "depth": 10 },
        "max_weight": 10,
        "count": 250
    });
    let large_stock_request = actix_test::TestRequest::post()
        .uri("/api/v1/box")
        .set_json(&large_stock_payload)
        .to_request();
    let large_stock_response = actix_test::call_service(&app, large_stock_request).await;
    assert_eq!(large_stock_response.status(), StatusCode::OK);
    let overflow_request = actix_test::TestRequest::post()
        .uri("/api/v1/box")
        .set_json(json!({
            "bounds": { "width": 10, "height": 10, "depth": 10 },
            "max_weight": 10,
            "count": 10
        }))
        .to_request();
    let overflow_response = actix_test::call_service(&app, overflow_request).await;
    assert_eq!(overflow_response.status(), StatusCode::BAD_REQUEST);

    let delete_request = actix_test::TestRequest::delete()
        .uri(&format!("/api/v1/box/{box_path}"))
        .to_request();
    let deleted: Boxes = actix_test::call_and_read_body_json(&app, delete_request).await;
    assert_eq!(deleted, updated);

    let missing_request = actix_test::TestRequest::get()
        .uri(&format!("/api/v1/box/{box_path}"))
        .to_request();
    let missing_response = actix_test::call_service(&app, missing_request).await;
    assert_eq!(missing_response.status(), StatusCode::NO_CONTENT);
}

#[derive(Serialize)]
struct OrderPayload {
    products: Vec<(RecordId, usize)>,
    package_box: Vec<(RecordId, usize)>,
    status: OrderStatus,
}

#[actix_web::test]
async fn orders_crud_preserves_record_references_and_quantities() {
    let db = database().await;
    let app = actix_test::init_service(
        App::new().app_data(database_data(&db)).service(
            web::scope("/api/v1")
                .service(create_new_order_with_data)
                .service(get_all_orders)
                .service(get_order_by_id)
                .service(update_order_by_id)
                .service(delete_order_by_id),
        ),
    )
    .await;
    let product_id = RecordId::new("products", RecordIdKey::uuid());
    let box_id = RecordId::new("boxes", RecordIdKey::uuid());

    let create_request = actix_test::TestRequest::post()
        .uri("/api/v1/order")
        .set_json(OrderPayload {
            products: vec![(product_id.clone(), 3)],
            package_box: Vec::new(),
            status: OrderStatus::NotPacked,
        })
        .to_request();
    let created: Order = actix_test::call_and_read_body_json(&app, create_request).await;
    assert_eq!(created.get_products(), [(product_id.clone(), 3)].as_slice());
    assert_eq!(created.get_status(), &OrderStatus::NotPacked);
    let order_path = uuid_path(created.get_id());

    let update_request = actix_test::TestRequest::post()
        .uri(&format!("/api/v1/order/update/{order_path}"))
        .set_json(OrderPayload {
            products: vec![(product_id.clone(), 2)],
            package_box: vec![(box_id.clone(), 1)],
            status: OrderStatus::Packed,
        })
        .to_request();
    let updated: Order = actix_test::call_and_read_body_json(&app, update_request).await;
    assert_eq!(updated.get_products(), [(product_id, 2)].as_slice());
    assert_eq!(updated.get_package_box(), [(box_id, 1)].as_slice());
    assert_eq!(updated.get_status(), &OrderStatus::Packed);

    let get_request = actix_test::TestRequest::get()
        .uri(&format!("/api/v1/order/{order_path}"))
        .to_request();
    let fetched: Order = actix_test::call_and_read_body_json(&app, get_request).await;
    assert_eq!(fetched, updated);

    let list_request = actix_test::TestRequest::get()
        .uri("/api/v1/orders")
        .to_request();
    let orders: Vec<Order> = actix_test::call_and_read_body_json(&app, list_request).await;
    assert_eq!(orders, vec![updated.clone()]);

    let invalid_id_request = actix_test::TestRequest::get()
        .uri("/api/v1/order/not-a-uuid")
        .to_request();
    let invalid_id_response = actix_test::call_service(&app, invalid_id_request).await;
    assert_eq!(invalid_id_response.status(), StatusCode::BAD_REQUEST);

    let delete_request = actix_test::TestRequest::delete()
        .uri(&format!("/api/v1/order/{order_path}"))
        .to_request();
    let deleted: Order = actix_test::call_and_read_body_json(&app, delete_request).await;
    assert_eq!(deleted, updated);
}

#[actix_web::test]
async fn registration_login_and_session_restoration_work_end_to_end() {
    let db = database().await;
    let key_pair = KeyPair::generate();
    let token_signer = TokenSigner::<User, Ed25519>::new()
        .signing_key(key_pair.sk.clone())
        .algorithm(Ed25519)
        .build()
        .unwrap();
    let authority = Authority::<User, Ed25519, _, _>::new()
        .refresh_authorizer(|| async move { Ok(()) })
        .token_signer(Some(token_signer.clone()))
        .verifying_key(key_pair.pk)
        .build()
        .unwrap();
    let app = actix_test::init_service(
        App::new()
            .app_data(database_data(&db))
            .service(regster_user)
            .service(validate_email_field)
            .service(validate_password_field)
            .service(authorize_user)
            .use_jwt(
                authority,
                web::scope("/api/v1")
                    .service(restore_session)
                    .service(logout),
            ),
    )
    .await;

    let missing_password_request = actix_test::TestRequest::post()
        .uri("/api/v1/register")
        .set_json(json!({
            "email": "missing@gmail.com",
            "password": null,
            "verify_password": null,
            "registration_type": "Default",
            "psuid": null
        }))
        .to_request();
    let missing_password_response = actix_test::call_service(&app, missing_password_request).await;
    assert_eq!(missing_password_response.status(), StatusCode::BAD_REQUEST);

    let missing_psuid_request = actix_test::TestRequest::post()
        .uri("/api/v1/register")
        .set_json(json!({
            "email": "missing-psuid@yandex.ru",
            "password": null,
            "verify_password": null,
            "registration_type": "YandexID",
            "psuid": null
        }))
        .to_request();
    let missing_psuid_response = actix_test::call_service(&app, missing_psuid_request).await;
    assert_eq!(missing_psuid_response.status(), StatusCode::BAD_REQUEST);

    let invalid_email_request = actix_test::TestRequest::post()
        .uri("/api/v1/validate_email")
        .set_json(json!({ "data": "not-an-email" }))
        .to_request();
    let invalid_email_response = actix_test::call_service(&app, invalid_email_request).await;
    assert_eq!(invalid_email_response.status(), StatusCode::CONFLICT);

    let invalid_password_request = actix_test::TestRequest::post()
        .uri("/api/v1/validate_password")
        .set_json(json!({ "data": "short" }))
        .to_request();
    let invalid_password_response = actix_test::call_service(&app, invalid_password_request).await;
    assert_eq!(invalid_password_response.status(), StatusCode::CONFLICT);

    let register_request = actix_test::TestRequest::post()
        .uri("/api/v1/register")
        .set_json(json!({
            "email": "backend@gmail.com",
            "password": "Password1!",
            "verify_password": "Password1!",
            "registration_type": "Default",
            "psuid": null
        }))
        .to_request();
    let register_response = actix_test::call_service(&app, register_request).await;
    assert_eq!(register_response.status(), StatusCode::OK);
    assert_eq!(register_response.response().cookies().count(), 2);

    let unknown_user_request = actix_test::TestRequest::post()
        .uri("/api/v1/auth")
        .set_json(json!({
            "email": "unknown@gmail.com",
            "password": "Password1!",
            "psuid": null
        }))
        .to_request();
    let unknown_user_response = actix_test::call_service(&app, unknown_user_request).await;
    assert_eq!(unknown_user_response.status(), StatusCode::UNAUTHORIZED);

    let wrong_password_request = actix_test::TestRequest::post()
        .uri("/api/v1/auth")
        .set_json(json!({
            "email": "backend@gmail.com",
            "password": "WrongPassword1!",
            "psuid": null
        }))
        .to_request();
    let wrong_password_response = actix_test::call_service(&app, wrong_password_request).await;
    assert_eq!(wrong_password_response.status(), StatusCode::UNAUTHORIZED);

    let login_request = actix_test::TestRequest::post()
        .uri("/api/v1/auth")
        .set_json(json!({
            "email": "backend@gmail.com",
            "password": "Password1!",
            "psuid": null
        }))
        .to_request();
    let login_response = actix_test::call_service(&app, login_request).await;
    assert_eq!(login_response.status(), StatusCode::OK);
    let login_cookies: Vec<_> = login_response
        .response()
        .cookies()
        .map(|cookie| cookie.into_owned())
        .collect();
    assert_eq!(login_cookies.len(), 2);

    let mut session_request = actix_test::TestRequest::get().uri("/api/v1/session");
    for cookie in &login_cookies {
        session_request = session_request.cookie(cookie.clone());
    }
    let restored: User =
        actix_test::call_and_read_body_json(&app, session_request.to_request()).await;
    assert_eq!(restored.get_email(), "backend@gmail.com");
    assert_eq!(restored.get_role(), &UserRoles::Employer);

    let mut logout_request = actix_test::TestRequest::post().uri("/api/v1/logout");
    for cookie in &login_cookies {
        logout_request = logout_request.cookie(cookie.clone());
    }
    let logout_response = actix_test::call_service(&app, logout_request.to_request()).await;
    assert_eq!(logout_response.status(), StatusCode::OK);
    let removed_cookies: Vec<_> = logout_response.response().cookies().collect();
    assert_eq!(removed_cookies.len(), 2);
    assert!(
        removed_cookies
            .iter()
            .all(|cookie| cookie.value().is_empty())
    );

    let mut revoked_session_request = actix_test::TestRequest::get().uri("/api/v1/session");
    for cookie in &login_cookies {
        revoked_session_request = revoked_session_request.cookie(cookie.clone());
    }
    let revoked_session_response =
        actix_test::call_service(&app, revoked_session_request.to_request()).await;
    assert_eq!(revoked_session_response.status(), StatusCode::UNAUTHORIZED);

    let ghost = User::default().set_email("ghost@gmail.com");
    let ghost_cookie = token_signer.create_access_cookie(&ghost).unwrap();
    let missing_session_request = actix_test::TestRequest::get()
        .uri("/api/v1/session")
        .cookie(ghost_cookie)
        .to_request();
    let missing_session_response = actix_test::call_service(&app, missing_session_request).await;
    assert_eq!(missing_session_response.status(), StatusCode::UNAUTHORIZED);

    let no_token_request = actix_test::TestRequest::get()
        .uri("/api/v1/session")
        .to_request();
    let no_token_error = actix_test::try_call_service(&app, no_token_request)
        .await
        .expect_err("request without JWT should be rejected");
    assert_eq!(
        no_token_error.as_response_error().status_code(),
        StatusCode::UNAUTHORIZED
    );

    let yandex_register_request = actix_test::TestRequest::post()
        .uri("/api/v1/register")
        .set_json(json!({
            "email": "backend@yandex.ru",
            "password": null,
            "verify_password": null,
            "registration_type": "YandexID",
            "psuid": "yandex-user-1"
        }))
        .to_request();
    let yandex_register_response = actix_test::call_service(&app, yandex_register_request).await;
    assert_eq!(yandex_register_response.status(), StatusCode::OK);

    let wrong_psuid_request = actix_test::TestRequest::post()
        .uri("/api/v1/auth")
        .set_json(json!({
            "email": "backend@yandex.ru",
            "password": null,
            "psuid": "wrong"
        }))
        .to_request();
    let wrong_psuid_response = actix_test::call_service(&app, wrong_psuid_request).await;
    assert_eq!(wrong_psuid_response.status(), StatusCode::UNAUTHORIZED);

    let correct_psuid_request = actix_test::TestRequest::post()
        .uri("/api/v1/auth")
        .set_json(json!({
            "email": "backend@yandex.ru",
            "password": null,
            "psuid": "yandex-user-1"
        }))
        .to_request();
    let correct_psuid_response = actix_test::call_service(&app, correct_psuid_request).await;
    assert_eq!(correct_psuid_response.status(), StatusCode::OK);
}

#[actix_web::test]
async fn jwt_role_guard_allows_only_super_admin_to_mutate_resources() {
    let db = database().await;
    let key_pair = KeyPair::generate();
    let token_signer = TokenSigner::<User, Ed25519>::new()
        .signing_key(key_pair.sk.clone())
        .algorithm(Ed25519)
        .build()
        .unwrap();
    let authority = Authority::<User, Ed25519, _, _>::new()
        .refresh_authorizer(|| async move { Ok(()) })
        .token_signer(Some(token_signer.clone()))
        .verifying_key(key_pair.pk)
        .build()
        .unwrap();
    let app = actix_test::init_service(
        App::new().app_data(database_data(&db)).use_jwt(
            authority,
            web::scope("/api/v1").service(get_all_products).service(
                web::scope("")
                    .guard(UserRole)
                    .service(create_new_products_with_data),
            ),
        ),
    )
    .await;
    let product_json = json!({
        "name": "guarded product",
        "marker": null,
        "bounds": { "width": 10, "height": 20, "depth": 30 },
        "weight": 4,
        "count": 10
    });
    let employer = User::default().set_email("employer@gmail.com");
    let employer_cookie = token_signer.create_access_cookie(&employer).unwrap();

    let read_request = actix_test::TestRequest::get()
        .uri("/api/v1/products")
        .cookie(employer_cookie.clone())
        .to_request();
    let read_response = actix_test::call_service(&app, read_request).await;
    assert_eq!(read_response.status(), StatusCode::OK);

    let denied_request = actix_test::TestRequest::post()
        .uri("/api/v1/product")
        .cookie(employer_cookie)
        .set_json(&product_json)
        .to_request();
    let denied_response = actix_test::call_service(&app, denied_request).await;
    assert_eq!(denied_response.status(), StatusCode::NOT_FOUND);

    let super_admin = User::default()
        .set_email("admin@gmail.com")
        .set_role(UserRoles::SuperAdmin);
    let admin_cookie = token_signer.create_access_cookie(&super_admin).unwrap();
    let allowed_request = actix_test::TestRequest::post()
        .uri("/api/v1/product")
        .cookie(admin_cookie)
        .set_json(&product_json)
        .to_request();
    let created: Product = actix_test::call_and_read_body_json(&app, allowed_request).await;
    assert_eq!(created.get_name(), "guarded product");
}
