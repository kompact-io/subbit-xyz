use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// User action
/// New actions should be listed below
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Iou,
    Register,
    Auth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode)]
pub struct Config {
    #[n(0)]
    pub iou: u64,
    #[n(1)]
    pub reg: u64,
    #[n(2)]
    pub auth: u64,
}

impl Config {
    pub fn cost(&self, action: Action) -> u64 {
        match action {
            Action::Iou => self.iou,
            Action::Register => self.reg,
            Action::Auth => self.auth,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        // 3 registers, 10 auths, or 5 ious in a row exhausts a 30_000 capacity
        Self {
            iou: 6_000,
            reg: 10_000,
            auth: 3_000,
        }
    }
}
