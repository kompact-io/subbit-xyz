use minicbor::{Decode, Encode};
use subbit_l2_dto::MacTtl as Mac;

/// Server-observed state, everything a caller needs to persist/restore to
/// resume a session across a restart (e.g. the CLI, where `spend` and
/// `response` run as separate processes).
#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct Cache {
    #[n(0)]
    spent: u64,
    #[n(1)]
    committed: Option<u64>,
    #[n(2)]
    mac: Option<Mac>,
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

    pub fn mac(&self) -> Option<&Mac> {
        self.mac.as_ref()
    }

    pub fn set_spent(&mut self, spent: u64) {
        self.spent = spent;
    }

    pub fn set_spenable(&mut self, spendable: u64) {
        if let Some(committed) = self.committed {
            self.spent = committed.saturating_sub(spendable);
        }
    }

    pub fn set_committed(&mut self, committed: u64) {
        self.committed = Some(committed);
    }

    pub fn set_mac(&mut self, mac: Mac) {
        self.mac = Some(mac);
    }

    pub fn clear_mac(&mut self) {
        self.mac = None;
    }
}

#[cfg(test)]
mod tests {
    use subbit_core::{Duration, MacSig, Tag, VerifyingKey};
    use subbit_l2_dto::KeyTagTtl;

    use super::*;

    fn mac() -> Mac {
        Mac {
            body: KeyTagTtl {
                key: VerifyingKey::from([9u8; 32]),
                tag: Tag::from(b"t".to_vec()),
                ttl: Duration::from_secs(60),
            },
            sig: MacSig::from([0u8; 20]),
        }
    }

    #[test]
    fn starts_with_nothing_committed_or_spent() {
        let cache = Cache::new();
        assert_eq!(cache.spent(), 0);
        assert_eq!(cache.committed(), None);
        assert!(cache.mac().is_none());
    }

    #[test]
    fn set_spenable_derives_spent_from_committed_minus_spendable() {
        let mut cache = Cache::new();
        cache.set_committed(1_000);
        cache.set_spenable(300);
        assert_eq!(cache.spent(), 700);
    }

    #[test]
    fn set_spenable_saturates_rather_than_underflowing() {
        let mut cache = Cache::new();
        cache.set_committed(100);
        cache.set_spenable(300);
        assert_eq!(cache.spent(), 0);
    }

    #[test]
    fn set_spenable_is_a_noop_before_anything_is_committed() {
        let mut cache = Cache::new();
        cache.set_spenable(300);
        assert_eq!(cache.spent(), 0);
    }

    #[test]
    fn clear_mac_drops_a_cached_mac() {
        let mut cache = Cache::new();
        cache.set_mac(mac());
        assert!(cache.mac().is_some());
        cache.clear_mac();
        assert!(cache.mac().is_none());
    }
}
