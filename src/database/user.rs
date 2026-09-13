use crate::database::registration_type::{self, RegistrationType};
use actix_jwt_auth_middleware::FromRequest;
use serde::{Deserialize, Serialize};
use std::default::Default;
use surrealdb::types::SurrealValue;

#[derive(Serialize, Deserialize, SurrealValue, Default, FromRequest, Clone)]
pub struct User {
    email: String,
    #[serde(skip_serializing, default)]
    password: Option<String>,
    registration_type: RegistrationType,
    psuid: Option<String>,
    role: UserRoles,
}

#[derive(Serialize, Deserialize, SurrealValue, Default, Clone, Debug, PartialEq)]
pub enum UserRoles {
    SuperAdmin,
    #[default]
    Employer,
}

impl User {
    pub fn get_email(&self) -> &str {
        &self.email
    }

    pub fn get_hash(&self) -> &Option<String> {
        &self.password
    }

    pub fn get_psuid(&self) -> &Option<String> {
        &self.psuid
    }

    pub fn get_registration_type(&self) -> &RegistrationType {
        &self.registration_type
    }

    pub fn get_role(&self) -> &UserRoles {
        &self.role
    }

    pub fn set_email(mut self, email: &str) -> Self {
        self.email = email.to_string();
        self
    }

    pub fn set_hash(mut self, hash: &str) -> Self {
        self.password = Some(hash.to_string());
        self
    }

    pub fn set_psuid(mut self, psuid: &str) -> Self {
        self.psuid = Some(psuid.to_string());
        self
    }

    pub fn set_registration_type(
        mut self,
        registration_type: registration_type::RegistrationType,
    ) -> Self {
        self.registration_type = registration_type;
        self
    }

    #[cfg(test)]
    pub fn set_role(mut self, role: UserRoles) -> Self {
        self.role = role;
        self
    }

    pub fn ready_for_register(&self) -> bool {
        if self.password.is_some()
            && self.registration_type == registration_type::RegistrationType::Default
        {
            true
        } else {
            self.psuid.is_some()
                && self.registration_type == registration_type::RegistrationType::YandexID
        }
    }
}
