use std::str::FromStr;

use surrealdb::types::{RecordId, RecordIdKey, Uuid};

pub mod algorithm;
pub mod auth;
pub mod boxes;
pub mod orders;
pub mod products;
pub mod register;

pub(crate) fn parse_uuid_record_id(table: &str, value: &str) -> Result<RecordId, String> {
    Uuid::from_str(value)
        .map(|uuid| RecordId::new(table, RecordIdKey::Uuid(uuid)))
        .map_err(|error| format!("invalid record id: {error}"))
}
