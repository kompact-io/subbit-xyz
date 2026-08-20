use minicbor::{Decode, Encode};
use subbit_core::{Mac, envelope::Body};

/// Server-observed state — everything a caller needs to persist/restore to
/// resume a session across a restart (e.g. the CLI, where `spend` and
/// `response` run as separate processes).
#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct Cache {
    #[n(0)]
    spent: u64,
    #[n(1)]
    committed: Option<u64>,
    #[n(2)]
    mac: Option<Mac<Body>>,
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spent(&self) -> u64 {
        self.spent
    }

    pub fn committed(&self) -> Option<u64> {
        self.committed
    }

    pub fn mac(&self) -> Option<&Mac<Body>> {
        self.mac.as_ref()
    }

    pub fn set_spent(&mut self, spent: u64) {
        self.spent = spent;
    }

    pub fn set_committed(&mut self, committed: u64) {
        self.committed = Some(committed);
    }

    pub fn set_mac(&mut self, mac: Mac<Body>) {
        self.mac = Some(mac);
    }
}
