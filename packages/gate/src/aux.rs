use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct Aux {
    #[n(0)]
    is_active: bool,
}

impl Default for Aux {
    fn default() -> Self {
        Self { is_active: true }
    }
}

impl Aux {
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn deactive(&mut self) {
        self.is_active = false
    }
}
