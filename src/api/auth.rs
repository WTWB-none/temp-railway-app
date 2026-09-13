use actix_jwt_auth_middleware::TokenSigner;
use actix_web::{
    HttpRequest, HttpResponse, Responder,
    cookie::Cookie,
    get, post,
    web::{Data, Json},
};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use jwt_compact::alg::Ed25519;
use serde::Deserialize;

use crate::{
    GlobalDBHandler,
    database::{jwt_session::JwtSession, registration_type, user::User},
};

#[derive(Deserialize)]
struct AuthData {
    email: String,
    password: Option<String>,
    psuid: Option<String>,
}

fn removal_cookie(name: &str) -> Cookie<'static> {
    let mut cookie = Cookie::build(name.to_owned(), "")
        .path("/api/v1")
        .secure(true)
        .http_only(true)
        .finish();
    cookie.make_removal();
    cookie
}

pub(crate) async fn issue_session(
    user: &User,
    data: &GlobalDBHandler,
    token_signer: &TokenSigner<User, Ed25519>,
) -> Result<(Cookie<'static>, Cookie<'static>), String> {
    let access_cookie = token_signer
        .create_access_cookie(user)
        .map_err(|error| error.to_string())?;
    let refresh_cookie = token_signer
        .create_refresh_cookie(user)
        .map_err(|error| error.to_string())?;

    let session = JwtSession::new(
        user.get_email(),
        access_cookie.value(),
        refresh_cookie.value(),
    );
    let created = data
        .handler
        .create::<Option<JwtSession>>("jwt_sessions")
        .content(session)
        .await
        .map_err(|error| error.to_string())?;

    if created.is_none() {
        return Err("JWT session was not saved".to_owned());
    }

    Ok((access_cookie, refresh_cookie))
}

#[post("/api/v1/auth")]
async fn authorize_user(
    info: Json<AuthData>,
    data: Data<GlobalDBHandler>,
    token_signer: Data<TokenSigner<User, Ed25519>>,
) -> impl Responder {
    let mut result = match data
        .handler
        .query("SELECT * FROM users WHERE email = $email LIMIT 1;")
        .bind(("email", info.email.clone()))
        .await
    {
        Ok(result) => result,
        Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
    };

    let user = match result.take::<Option<User>>(0) {
        Ok(Some(user)) => user,
        Ok(None) => return HttpResponse::Unauthorized().body("user not found"),
        Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
    };

    match user.get_registration_type() {
        registration_type::RegistrationType::Default => {
            let (Some(password), Some(password_hash)) =
                (info.password.as_ref(), user.get_hash().as_ref())
            else {
                return HttpResponse::Unauthorized().body("password does not provided");
            };
            let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
                return HttpResponse::InternalServerError().body("stored password hash is invalid");
            };
            if Argon2::default()
                .verify_password(password.as_bytes(), &parsed_hash)
                .is_err()
            {
                return HttpResponse::Unauthorized().body("password does not match");
            }
        }
        registration_type::RegistrationType::YandexID => {
            if info.psuid.as_ref() != user.get_psuid().as_ref() {
                return HttpResponse::Unauthorized().body("psuid does not match");
            }
        }
    }

    match issue_session(&user, data.get_ref(), token_signer.get_ref()).await {
        Ok((access_cookie, refresh_cookie)) => HttpResponse::Ok()
            .cookie(access_cookie)
            .cookie(refresh_cookie)
            .json(&user),
        Err(error) => HttpResponse::InternalServerError().body(error),
    }
}

#[get("/session")]
pub async fn restore_session(
    claims: User,
    request: HttpRequest,
    data: Data<GlobalDBHandler>,
    token_signer: Data<TokenSigner<User, Ed25519>>,
) -> impl Responder {
    let access_token = request
        .cookie(token_signer.access_token_name())
        .map(|cookie| cookie.value().to_owned())
        .unwrap_or_default();
    let refresh_token = request
        .cookie(token_signer.refresh_token_name())
        .map(|cookie| cookie.value().to_owned())
        .unwrap_or_default();

    let mut session_result = match data
        .handler
        .query(
            "SELECT * FROM jwt_sessions \
             WHERE user_email = $email \
             AND (access_token = $access_token OR refresh_token = $refresh_token) \
             LIMIT 1;",
        )
        .bind(("email", claims.get_email().to_owned()))
        .bind(("access_token", access_token))
        .bind(("refresh_token", refresh_token))
        .await
    {
        Ok(result) => result,
        Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
    };

    match session_result.take::<Option<JwtSession>>(0) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return HttpResponse::Unauthorized()
                .cookie(removal_cookie(token_signer.access_token_name()))
                .cookie(removal_cookie(token_signer.refresh_token_name()))
                .body("session not found");
        }
        Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
    }

    let mut user_result = match data
        .handler
        .query("SELECT * FROM users WHERE email = $email LIMIT 1;")
        .bind(("email", claims.get_email().to_owned()))
        .await
    {
        Ok(result) => result,
        Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
    };

    match user_result.take::<Option<User>>(0) {
        Ok(Some(user)) => HttpResponse::Ok().json(user),
        Ok(None) => HttpResponse::Unauthorized().body("user not found"),
        Err(error) => HttpResponse::InternalServerError().body(error.to_string()),
    }
}

#[post("logout")]
pub async fn logout(
    claims: User,
    request: HttpRequest,
    data: Data<GlobalDBHandler>,
    token_signer: Data<TokenSigner<User, Ed25519>>,
) -> impl Responder {
    let access_token = request
        .cookie(token_signer.access_token_name())
        .map(|cookie| cookie.value().to_owned())
        .unwrap_or_default();
    let refresh_token = request
        .cookie(token_signer.refresh_token_name())
        .map(|cookie| cookie.value().to_owned())
        .unwrap_or_default();

    let result = data
        .handler
        .query(
            "DELETE jwt_sessions \
             WHERE user_email = $email \
             AND (access_token = $access_token OR refresh_token = $refresh_token);",
        )
        .bind(("email", claims.get_email().to_owned()))
        .bind(("access_token", access_token))
        .bind(("refresh_token", refresh_token))
        .await;

    let deletion_error = match result {
        Ok(result) => result.check().err(),
        Err(error) => Some(error),
    };
    let (mut response, body) = if deletion_error.is_some() {
        (
            HttpResponse::InternalServerError(),
            "could not revoke session",
        )
    } else {
        (HttpResponse::Ok(), "logout")
    };

    response
        .cookie(removal_cookie(token_signer.access_token_name()))
        .cookie(removal_cookie(token_signer.refresh_token_name()))
        .body(body)
}
