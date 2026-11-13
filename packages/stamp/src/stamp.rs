//! Main stamp logic handling.

use subbit_core::{Duration, Instant};
use subbit_l2_dto::{Catalog as CatalogDto, ChannelStatus, Commit, MacTtl, PopMacTtl, Request};

use crate::catalog::{Catalog, CatalogError, Charge, MissDecision, MissPolicy};
use crate::{Cache, Config, Id, now};

#[derive(Debug, thiserror::Error)]
pub enum ResponseError {
    #[error("iou signature did not verify")]
    BadSignature,
}

/// Behaviour of stamp output
#[derive(Default)]
enum Mode {
    /// Standard (default): send bare ByPopMac while committed >= spent + cost;
    /// only mint a fresh Iou (ById) once that runway runs out.
    #[default]
    Standard,
    ById,
}

fn empty_catalog() -> Catalog {
    Catalog::new(CatalogDto {
        resources: Vec::new(),
    })
    .expect("an empty catalog never fails to index")
}

pub struct Stamp {
    id: Id,
    ttl_relative: Duration,
    cache: Cache,
    mode: Mode,
    catalog: Catalog,
}

impl Stamp {
    pub fn new(id: Id, ttl_relative: Duration) -> Self {
        Self {
            id,
            ttl_relative,
            cache: Cache::new(),
            mode: Mode::default(),
            catalog: empty_catalog(),
        }
    }

    /// Resume a session from persisted state.
    pub fn from_parts(config: Config, cache: Cache) -> Self {
        let catalog = cache
            .catalog()
            .cloned()
            .map(|dto| Catalog::new(dto).unwrap_or_else(|_| empty_catalog()))
            .unwrap_or_else(empty_catalog);
        Self {
            id: Id::from(config.id),
            ttl_relative: config.ttl_relative,
            cache,
            mode: Mode::default(),
            catalog,
        }
    }

    pub fn by_id(mut self) -> Self {
        self.mode = Mode::ById;
        self
    }

    pub fn id(&self) -> &Id {
        &self.id
    }

    pub fn ttl_relative(&self) -> Duration {
        self.ttl_relative
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn spent(&self) -> u64 {
        self.cache.spent()
    }

    pub fn committed(&self) -> Option<u64> {
        self.cache.committed()
    }

    pub fn set_committed(&mut self, committed: u64) {
        self.cache.set_committed(committed);
    }

    pub fn catalog(&self) -> Option<&CatalogDto> {
        self.cache.catalog()
    }

    pub fn set_catalog(&mut self, catalog: CatalogDto) -> Result<(), CatalogError> {
        self.catalog = Catalog::new(catalog.clone())?;
        self.cache.set_catalog(catalog);
        Ok(())
    }

    pub fn resolve_charge(
        &mut self,
        path: &str,
        policy: &dyn MissPolicy,
    ) -> Result<Charge, CatalogError> {
        if let Some(cost) = self.catalog.cost(path) {
            return Ok(Charge::Priced(cost));
        }
        Ok(match policy.on_miss(path) {
            MissDecision::Reject => Charge::Reject,
            MissDecision::Unpriced => Charge::Unpriced,
            MissDecision::Once(amount) => Charge::Priced(amount),
            MissDecision::Learn(pattern, info) => {
                let cost = info.cost.quote();
                self.catalog.insert(pattern, info)?;
                Charge::Priced(cost)
            }
        })
    }

    fn now(&self) -> Instant {
        now().unwrap_or(Instant::from_secs(0))
    }

    pub fn auth(&self) -> PopMacTtl {
        match self.cache.mac() {
            Some(mac) => PopMacTtl::Mac(mac.clone()),
            None => PopMacTtl::Pop(self.id.pop(self.now() + self.ttl_relative)),
        }
    }

    /// Mints an `Iou` covering `cost` on top of what's already spent,
    /// committing/spending it eagerly (see `apply_status`'s own doc
    /// comment on why that's safe).
    ///
    /// The `Iou` doubles as the request's authorization (`ById` carries
    /// no `Pop`/`Mac`), so it must be *strictly newer* than the one the
    /// server already holds or it's an `OldIou` reject. Whenever what's
    /// already committed covers `spent + cost` (a resync off a server
    /// status can leave `spent` well below `committed`), the amount is
    /// bumped to `committed + 1` instead: one lovelace over-commit buys
    /// a fresh signature, and the runway it covers is still consumed by
    /// `spent` as normal.
    fn commit(&mut self, cost: u64) -> Commit {
        let required = self.spent() + cost;
        let committed = self.committed().unwrap_or(0);
        let amount = if required > committed {
            required
        } else {
            committed + 1
        };
        let iou = self.id.iou(amount);
        self.cache.set_committed(amount);
        self.cache.set_spent(required);
        Commit::Iou(iou)
    }

    pub fn by_id_request(&mut self, cost: u64) -> Request {
        let commit = self.commit(cost);
        Request::ById {
            channel_id: self.id.channel_id(),
            commit,
        }
    }

    /// New iou not required
    pub fn auth_only_request(&mut self, cost: u64) -> Request {
        self.cache.set_spent(self.spent() + cost);
        Request::ByPopMac {
            auth: self.auth(),
            commit: None,
        }
    }

    pub fn standard_request(&mut self, cost: u64) -> Request {
        if self.committed().unwrap_or(0) >= self.spent() + cost {
            self.auth_only_request(cost)
        } else {
            self.by_id_request(cost)
        }
    }

    pub(crate) fn next_request(&mut self, cost: u64) -> Request {
        match self.mode {
            Mode::ById => self.by_id_request(cost),
            Mode::Standard => self.standard_request(cost),
        }
    }

    pub(crate) fn apply_status(
        &mut self,
        status: ChannelStatus,
        mac: Option<MacTtl>,
    ) -> Result<(), ResponseError> {
        let current = self.cache.committed().unwrap_or(0);
        let amount = status.iou.amount();
        let is_new = amount > current;
        if is_new && !self.id.verify_iou(&status.iou) {
            return Err(ResponseError::BadSignature);
        }
        if is_new {
            self.cache.set_committed(amount);
        }
        if let Some(spendable) = status.spendable {
            self.cache.set_spenable(spendable);
        }
        if let Some(mac) = mac {
            self.cache.set_mac(mac);
        }
        Ok(())
    }

    pub(crate) fn clear_mac(&mut self) {
        self.cache.clear_mac();
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use subbit_core::{MacSig, Tag, VerifyingKey};
    use subbit_l2_dto::KeyTagTtl;

    use super::*;
    use crate::Id;

    fn id(seed: u8) -> Id {
        Id::new(
            SigningKey::from_bytes(&[seed; 32]),
            Tag::from(b"t".to_vec()),
        )
    }

    fn status_for(id: &Id, amount: u64, spendable: Option<u64>) -> ChannelStatus {
        ChannelStatus {
            iou: id.iou(amount),
            spendable,
            uncommitted: None,
        }
    }

    fn mac_ttl() -> MacTtl {
        MacTtl {
            body: KeyTagTtl {
                key: VerifyingKey::from([9u8; 32]),
                tag: Tag::from(b"t".to_vec()),
                expires_at: Instant::from_secs(60),
            },
            sig: MacSig::from([0u8; 20]),
        }
    }

    #[test]
    fn next_request_sends_an_iou_when_nothing_is_committed_yet() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        let payload = stamp.next_request(50);
        match payload {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 50),
            other => panic!("expected ById Iou, got {other:?}"),
        }
        assert_eq!(stamp.spent(), 50);
    }

    #[test]
    fn next_request_sends_pop_once_committed_already_covers_the_cost() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        stamp.set_committed(1_000);
        let payload = stamp.next_request(100);
        assert!(matches!(payload, Request::ByPopMac { commit: None, .. }));
        assert_eq!(stamp.spent(), 100);
    }

    #[test]
    fn next_request_requests_a_fresh_iou_once_committed_is_exhausted() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        stamp.set_committed(100);
        let first = stamp.next_request(60);
        assert!(matches!(first, Request::ByPopMac { commit: None, .. }));
        let second = stamp.next_request(60);
        match second {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 120),
            other => panic!("expected ById Iou, got {other:?}"),
        }
    }

    /// The loop `canary` hit live (`plan-canary-l2-oldiou-backoff.md`,
    /// 2026-11-12): a resync leaves `spent < committed`, and the next
    /// `ById` mint of `spent + cost` lands exactly on `committed`, which
    /// the server rejects as `OldIou` -- forever, since the rejection's
    /// status resyncs `spent` to the same place again.
    #[test]
    fn by_id_mints_a_strictly_newer_iou_when_committed_already_covers_the_cost() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60)).by_id();
        // Server holds 200, of which 100 is spent.
        let status = status_for(stamp.id(), 200, Some(100));
        stamp.apply_status(status, None).unwrap();
        assert_eq!(stamp.spent(), 100);
        let payload = stamp.next_request(100);
        match payload {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 201),
            other => panic!("expected ById Iou, got {other:?}"),
        }
        assert_eq!(stamp.committed(), Some(201));
        // The runway is consumed, not re-minted: spent tracks cost.
        assert_eq!(stamp.spent(), 200);
        // Once the runway is gone, the mint is sized by spent + cost again.
        match stamp.next_request(100) {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 300),
            other => panic!("expected ById Iou, got {other:?}"),
        }
        assert_eq!(stamp.spent(), 300);
    }

    #[test]
    fn apply_status_rejects_a_signature_from_a_different_key() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        let bad_status = status_for(&id(2), 100, Some(100));
        let err = stamp.apply_status(bad_status, None).unwrap_err();
        assert!(matches!(err, ResponseError::BadSignature));
        assert_eq!(stamp.committed(), None);
    }

    #[test]
    fn apply_status_commits_amount_spendable_and_mac() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        let status = status_for(stamp.id(), 7_000, Some(3_000));
        stamp.apply_status(status, Some(mac_ttl())).unwrap();
        assert_eq!(stamp.committed(), Some(7_000));
        assert!(matches!(stamp.auth(), PopMacTtl::Mac(_)));
    }

    #[test]
    fn auth_uses_pop_until_a_mac_is_cached() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        assert!(matches!(stamp.auth(), PopMacTtl::Pop(_)));
        let status = status_for(stamp.id(), 100, Some(100));
        stamp.apply_status(status, Some(mac_ttl())).unwrap();
        assert!(matches!(stamp.auth(), PopMacTtl::Mac(_)));
    }

    #[test]
    fn clear_mac_falls_back_to_pop() {
        let mut stamp = Stamp::new(id(1), Duration::from_secs(60));
        let status = status_for(stamp.id(), 100, Some(100));
        stamp.apply_status(status, Some(mac_ttl())).unwrap();
        assert!(matches!(stamp.auth(), PopMacTtl::Mac(_)));
        stamp.clear_mac();
        assert!(matches!(stamp.auth(), PopMacTtl::Pop(_)));
    }
}
