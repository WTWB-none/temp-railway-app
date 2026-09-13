use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};

#[derive(Serialize, Deserialize, SurrealValue, Clone, Debug, PartialEq)]
pub struct Order {
    id: RecordId,
    products: Vec<(RecordId, usize)>,
    package_box: Vec<(RecordId, usize)>,
    status: OrderStatus,
}

#[derive(Serialize, Deserialize, SurrealValue, Clone, Debug, PartialEq)]
pub enum OrderStatus {
    Packed,
    NotPacked,
}

impl Order {
    pub fn from_data(
        id: RecordId,
        products: Vec<(RecordId, usize)>,
        package_box: Vec<(RecordId, usize)>,
        status: OrderStatus,
    ) -> Self {
        Self {
            id,
            products,
            package_box,
            status,
        }
    }

    pub fn get_id(&self) -> &RecordId {
        &self.id
    }

    pub fn get_products(&self) -> &[(RecordId, usize)] {
        &self.products
    }

    #[cfg(test)]
    pub fn get_package_box(&self) -> &[(RecordId, usize)] {
        &self.package_box
    }

    #[cfg(test)]
    pub fn get_status(&self) -> &OrderStatus {
        &self.status
    }
}
