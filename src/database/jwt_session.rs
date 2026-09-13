use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Serialize, Deserialize, SurrealValue, Clone)]
pub struct JwtSession {
    user_email: String,
    access_token: String,
    refresh_token: String,
}

impl JwtSession {
    pub fn new(user_email: &str, access_token: &str, refresh_token: &str) -> Self {
        Self {
            user_email: user_email.to_owned(),
            access_token: access_token.to_owned(),
            refresh_token: refresh_token.to_owned(),
        }
    }
}
