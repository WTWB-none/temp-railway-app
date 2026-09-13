use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Serialize, Deserialize, SurrealValue, Default, PartialEq, Clone)]
pub enum RegistrationType {
    YandexID,
    #[default]
    Default,
}
