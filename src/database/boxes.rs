use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
#[derive(Serialize, Deserialize, SurrealValue, Clone, Debug, PartialEq)]
pub struct Boxes {
    id: RecordId,
    bounds: BoxBounds,
    max_weight: u16,
    count: u8,
    volume: u64,
}

impl Boxes {
    pub fn from_data(id: RecordId, bounds: BoxBounds, max_weight: u16, count: u8) -> Self {
        Self {
            id,
            bounds,
            max_weight,
            count,
            volume: bounds.get_volume(),
        }
    }

    pub fn increase_count(&self, new_count: u8) -> Option<Self> {
        self.count.checked_add(new_count).map(|count| Self {
            count,
            ..self.clone()
        })
    }

    pub fn get_id(&self) -> &RecordId {
        &self.id
    }

    pub fn get_bounds(&self) -> &BoxBounds {
        &self.bounds
    }

    pub fn get_max_weight(&self) -> u16 {
        self.max_weight
    }

    pub fn get_count(&self) -> u8 {
        self.count
    }
}

#[derive(Serialize, Deserialize, SurrealValue, Clone, PartialEq, Debug, Copy)]
pub struct BoxBounds {
    width: u16,
    height: u16,
    depth: u16,
}

impl BoxBounds {
    pub(self) fn get_volume(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * u64::from(self.depth)
    }

    pub fn get_width(&self) -> u16 {
        self.width
    }

    pub fn get_height(&self) -> u16 {
        self.height
    }

    pub fn get_depth(&self) -> u16 {
        self.depth
    }
}
