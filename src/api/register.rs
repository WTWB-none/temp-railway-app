use crate::database::{
    registration_type::{self, RegistrationType},
    user::User,
};
use actix_jwt_auth_middleware::TokenSigner;
use actix_web::{
    HttpResponse, Responder, post,
    web::{self, Json},
};
use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use jwt_compact::alg::Ed25519;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::api::auth::issue_session;
use crate::{GlobalDBHandler, traits::parsable_error::ParsableError};

#[derive(Serialize, Deserialize)]
struct UserRegisterData {
    email: String,
    password: Option<String>,
    verify_password: Option<String>,
    registration_type: RegistrationType,
    psuid: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct FieldValidator {
    data: String,
}

#[derive(Debug, PartialEq)]
enum EmailError {
    NotEmail,
    NotSupportedEmail,
}

impl ParsableError for EmailError {
    fn into_string(self) -> &'static str {
        match self {
            Self::NotEmail => "provided email is not an actual email",
            Self::NotSupportedEmail => "email provider is not yet supported",
        }
    }
}

#[derive(Debug, PartialEq)]
enum PasswordError {
    SpecialCharacter,
    Numbers,
    Letters,
    Length,
    Unexpected,
}

impl ParsableError for PasswordError {
    fn into_string(self) -> &'static str {
        match self {
            Self::SpecialCharacter => "password does not contain any special characters",
            Self::Numbers => "password does not contain any numbers",
            Self::Letters => "password does not contain any latin letters",
            Self::Length => "password is less then 8 characters",
            Self::Unexpected => "wtf was that?",
        }
    }
}

const EMAIL_LIST: [&str; 4] = ["gmail.com", "yandex.ru", "vk.com", "mail.ru"];
const MIN_PASSWORD_LENGTH: u8 = 8;

fn validate_email(email: String) -> Result<(), EmailError> {
    let regex = Regex::new(r"^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$").unwrap();
    match regex.is_match(&email) {
        true => {
            let (_, email_provider) = email.trim().split_once('@').unwrap();
            if !EMAIL_LIST.contains(&email_provider) {
                Err(EmailError::NotSupportedEmail)
            } else {
                Ok(())
            }
        }
        false => Err(EmailError::NotEmail),
    }
}

fn validate_password(password: String) -> Result<(), PasswordError> {
    let special_character_regex = Regex::new(r"[!@#$%^&*()_+\-=\[\]{};':\\|,.<>/?]").unwrap();
    let numbers_regex = Regex::new(r"[0-9]").unwrap();
    let letters_regex = Regex::new(r"[A-Za-z]").unwrap();
    match special_character_regex.is_match(&password)
        && numbers_regex.is_match(&password)
        && letters_regex.is_match(&password)
        && password.len() >= MIN_PASSWORD_LENGTH as usize
    {
        true => Ok(()),
        false => {
            if password.len() < MIN_PASSWORD_LENGTH as usize {
                Err(PasswordError::Length)
            } else if !special_character_regex.is_match(&password) {
                Err(PasswordError::SpecialCharacter)
            } else if !numbers_regex.is_match(&password) {
                Err(PasswordError::Numbers)
            } else if !letters_regex.is_match(&password) {
                Err(PasswordError::Letters)
            } else {
                Err(PasswordError::Unexpected)
            }
        }
    }
}

fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

#[post("api/v1/register")]
async fn regster_user(
    info: web::Json<UserRegisterData>,
    data: web::Data<GlobalDBHandler>,
    token: web::Data<TokenSigner<User, Ed25519>>,
) -> impl Responder {
    let mut error: Vec<String> = vec![];
    let user = match info.registration_type {
        registration_type::RegistrationType::Default => {
            let (Some(password), Some(verify_password)) =
                (info.password.as_deref(), info.verify_password.as_deref())
            else {
                return HttpResponse::BadRequest().body("password and verification are required");
            };
            if password != verify_password {
                error.push("passwords do not match".to_string());
            }
            let _ = validate_password(password.to_owned())
                .map_err(|e| error.push(e.into_string().to_string()));
            let _ = validate_email(info.email.clone())
                .map_err(|e| error.push(e.into_string().to_string()));

            if !error.is_empty() {
                return HttpResponse::BadRequest().json(&error);
            }

            let password_hash = match hash_password(password) {
                Ok(hash) => hash,
                Err(error) => return HttpResponse::InternalServerError().body(error.to_string()),
            };
            User::default()
                .set_hash(&password_hash)
                .set_email(&info.email)
        }
        registration_type::RegistrationType::YandexID => {
            let Some(psuid) = info.psuid.as_deref() else {
                return HttpResponse::BadRequest().body("psuid is required");
            };
            User::default()
                .set_registration_type(registration_type::RegistrationType::YandexID)
                .set_email(&info.email)
                .set_psuid(psuid)
        }
    };

    match user.ready_for_register() {
        true => {
            match data
                .handler
                .create::<Option<User>>("users")
                .content(user.clone())
                .await
            {
                Ok(_) => match issue_session(&user, data.get_ref(), token.get_ref()).await {
                    Ok((access_cookie, refresh_cookie)) => HttpResponse::Ok()
                        .cookie(access_cookie)
                        .cookie(refresh_cookie)
                        .body("registered_user"),
                    Err(error) => HttpResponse::InternalServerError().body(error),
                },
                Err(error) => HttpResponse::InternalServerError().body(error.to_string()),
            }
        }
        false => HttpResponse::InternalServerError().body("data incomplete"),
    }
}

#[post("/api/v1/validate_email")]
async fn validate_email_field(info: Json<FieldValidator>) -> impl Responder {
    let mut error: Vec<String> = vec![];
    let _ = validate_email(info.data.clone()).map_err(|e| error.push(e.into_string().to_string()));
    if error.is_empty() {
        HttpResponse::Ok().body("Ok")
    } else {
        HttpResponse::Conflict().json(&error)
    }
}

#[post("/api/v1/validate_password")]
pub async fn validate_password_field(info: Json<FieldValidator>) -> impl Responder {
    let mut error: Vec<String> = vec![];
    let _ =
        validate_password(info.data.clone()).map_err(|e| error.push(e.into_string().to_string()));
    if error.is_empty() {
        HttpResponse::Ok().body("Ok")
    } else {
        HttpResponse::Conflict().json(&error)
    }
}

#[cfg(test)]
mod test {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    use crate::api::register::{
        EmailError, PasswordError, hash_password, validate_email, validate_password,
    };

    #[test]
    fn check_email_valid() {
        let emails = vec![
            "temp@gmail.com",
            "temp@yandex.ru",
            "temp@vk.com",
            "temp@mail.ru",
        ];
        for email in emails {
            assert_eq!(Ok(()), validate_email(email.to_string()));
        }
    }

    #[test]
    fn check_is_not_email() {
        let email = "temp";
        assert_eq!(Err(EmailError::NotEmail), validate_email(email.to_string()));
    }

    #[test]
    fn rejects_email_with_trailing_data() {
        let email = "temp@gmail.com invalid";
        assert_eq!(Err(EmailError::NotEmail), validate_email(email.to_string()));
    }

    #[test]
    fn rejects_unsupported_email_provider() {
        let email = "temp@example.com";
        assert_eq!(
            Err(EmailError::NotSupportedEmail),
            validate_email(email.to_string())
        );
    }

    #[test]
    fn check_passwod_valid() {
        let password = "temppassword1!";
        assert_eq!(Ok(()), validate_password(password.to_string()));
    }

    #[test]
    fn check_password_without_letters() {
        let password = "12345678!";
        assert_eq!(
            Err(PasswordError::Letters),
            validate_password(password.to_string())
        );
    }

    #[test]
    fn check_password_without_numbers() {
        let password = "temppassword!";
        assert_eq!(
            Err(PasswordError::Numbers),
            validate_password(password.to_string())
        );
    }

    #[test]
    fn check_password_without_special_characters() {
        let password = "12345678a";
        assert_eq!(
            Err(PasswordError::SpecialCharacter),
            validate_password(password.to_string())
        );
    }

    #[test]
    fn check_password_with_not_enough_length() {
        let password = "12345a!";
        assert_eq!(
            Err(PasswordError::Length),
            validate_password(password.to_string())
        );
    }

    #[test]
    fn hashes_password_with_argon2() {
        let password = "temppassword1!";
        let hash = hash_password(password).unwrap();
        let parsed_hash = PasswordHash::new(&hash).unwrap();

        assert_ne!(hash, password);
        assert!(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed_hash)
                .is_ok()
        );
    }
}
