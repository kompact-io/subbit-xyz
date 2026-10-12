//! Dialect-agnostic core: decides what to send next (an `Iou` once
//! there's enough committed to cover it, a bare `PopMac` liveness
//! proof otherwise) and applies whatever the server reports back. No
//! wire framing here at all, that's each `dialect` module's job, the
//! same split `subbit-server`'s own `Ctx` vs `dialect::{vanilla,x402}`
//! makes on the provider side.

use serde::{Deserialize, Serialize};
use subbit_core::Duration;
use subbit_l2_dto::{ChannelStatus, Commit, MacTtl, PopMacTtl, Request};

use crate::{Account, Cache, account, now};

/// Proof of liveness: `subbit-l2-dto`'s own `PopMacTtl`, used as-is,
/// dialect-agnostic, both wire dialects carry it unchanged.
pub type Auth = PopMacTtl;

/// On-disk/wire shape for an issuer's identity: the permissive
/// `account::Config` plus the TTL used for proof-of-possession envelopes.
/// `Issuer` doesn't hold this directly, see `Issuer::from_parts`, which
/// resolves `account` into a real `Account` once and stores account/ttl
/// flattened from then on. This type exists purely for (de)serialization.
#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
pub struct Config {
    #[n(0)]
    pub account: account::Config,
    #[n(1)]
    pub ttl_relative: Duration,
}

impl Config {
    pub fn new(account: account::Config, ttl_relative: Duration) -> Self {
        Self {
            account,
            ttl_relative,
        }
    }
}

/// What the core itself can get wrong applying a decoded status:
/// each dialect's own `Error` wraps this for its own, additionally
/// wire-shaped, failure modes (a bad decode, a protocol-level
/// rejection).
#[derive(Debug, thiserror::Error)]
pub enum ResponseError {
    #[error("iou signature did not verify")]
    BadSignature,
}

pub struct Issuer {
    account: Account,
    ttl_relative: Duration,
    cache: Cache,
}

impl Issuer {
    pub fn new(account: Account, ttl_relative: Duration) -> Self {
        Self {
            account,
            ttl_relative,
            cache: Cache::new(),
        }
    }

    /// Resume a session from persisted wire `Config`/`Cache`. `config.account`
    /// (a permissive `account::Config`) is resolved into the real `Account`
    /// here, once, rather than carried around as the wire type.
    pub fn from_parts(config: Config, cache: Cache) -> Self {
        Self {
            account: Account::from(config.account),
            ttl_relative: config.ttl_relative,
            cache,
        }
    }

    pub fn account(&self) -> &Account {
        &self.account
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

    fn now(&self) -> Duration {
        // FIXME:: Rationalize
        now().unwrap_or(Duration::from_secs(0))
    }

    pub fn auth(&self) -> Auth {
        match self.cache.mac() {
            Some(mac) => Auth::Mac(mac.clone()),
            None => Auth::Pop(self.account.pop(self.now() + self.ttl_relative)),
        }
    }

    /// The next claim to send, committing `spent + cost`: an `Iou` if
    /// that's covered by what's already committed, a bare liveness
    /// proof (`PopMac`) otherwise. Dialect-agnostic, `Request` is the
    /// shared wire envelope both dialects use as-is; each `dialect`
    /// module only adds its own encoding (and header) around this.
    pub(crate) fn next_payload(&mut self, cost: u64) -> Request {
        let spent = self.spent();
        let required = spent + cost;
        let payload = if self.committed().unwrap_or(0) >= required {
            Request::ByPopMac {
                auth: self.auth(),
                commit: None,
            }
        } else {
            Request::ById {
                channel_id: self.account.channel_id(),
                commit: Commit::Iou(self.account.iou(required)),
            }
        };
        self.cache.set_spent(required);
        payload
    }

    /// Verify `status`'s iou against this account, then commit it (and
    /// `mac`, if present) to the cache. Commits from the verified iou's
    /// own amount, no local bookkeeping of what was requested needed.
    /// Dialect-agnostic: every dialect's successful (or informatively
    /// failed, e.g. x402's corrective 402) reply boils down to this.
    pub(crate) fn apply_status(
        &mut self,
        status: ChannelStatus,
        mac: Option<MacTtl>,
    ) -> Result<(), ResponseError> {
        if !self.account.verify_iou(&status.iou) {
            return Err(ResponseError::BadSignature);
        }
        self.cache.set_committed(status.iou.amount());
        if let Some(spendable) = status.spendable {
            self.cache.set_spenable(spendable);
        }
        if let Some(mac) = mac {
            self.cache.set_mac(mac);
        }
        Ok(())
    }

    /// Drop a cached session `Mac`, e.g. a dialect's own `Auth`
    /// rejection should assume the mac is stale and fall back to a
    /// fresh `Pop` next time. Only vanilla's adapter calls this today
    /// (x402's `Response.error` carries no typed `InvalidAuth` to
    /// react to the same way).
    #[cfg(feature = "vanilla")]
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
    use crate::Account;

    fn account(seed: u8) -> Account {
        Account::new(
            SigningKey::from_bytes(&[seed; 32]),
            Tag::from(b"t".to_vec()),
        )
    }

    fn status_for(account: &Account, amount: u64, spendable: Option<u64>) -> ChannelStatus {
        ChannelStatus {
            iou: account.iou(amount),
            spendable,
            uncommitted: None,
        }
    }

    fn mac_ttl() -> MacTtl {
        MacTtl {
            body: KeyTagTtl {
                key: VerifyingKey::from([9u8; 32]),
                tag: Tag::from(b"t".to_vec()),
                ttl: Duration::from_secs(60),
            },
            sig: MacSig::from([0u8; 20]),
        }
    }

    #[test]
    fn next_payload_sends_an_iou_when_nothing_is_committed_yet() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        let payload = issuer.next_payload(50);
        match payload {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 50),
            other => panic!("expected ById Iou, got {other:?}"),
        }
        assert_eq!(issuer.spent(), 50);
    }

    #[test]
    fn next_payload_sends_pop_once_committed_already_covers_the_cost() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        issuer.set_committed(1_000);
        let payload = issuer.next_payload(100);
        assert!(matches!(payload, Request::ByPopMac { commit: None, .. }));
        assert_eq!(issuer.spent(), 100);
    }

    #[test]
    fn next_payload_requests_a_fresh_iou_once_committed_is_exhausted() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        issuer.set_committed(100);
        let first = issuer.next_payload(60);
        assert!(matches!(first, Request::ByPopMac { commit: None, .. }));
        let second = issuer.next_payload(60);
        match second {
            Request::ById {
                commit: Commit::Iou(iou),
                ..
            } => assert_eq!(iou.amount(), 120),
            other => panic!("expected ById Iou, got {other:?}"),
        }
    }

    #[test]
    fn apply_status_rejects_a_signature_from_a_different_key() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        let bad_status = status_for(&account(2), 100, Some(100));
        let err = issuer.apply_status(bad_status, None).unwrap_err();
        assert!(matches!(err, ResponseError::BadSignature));
        assert_eq!(issuer.committed(), None);
    }

    #[test]
    fn apply_status_commits_amount_spendable_and_mac() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        let status = status_for(issuer.account(), 7_000, Some(3_000));
        issuer.apply_status(status, Some(mac_ttl())).unwrap();
        assert_eq!(issuer.committed(), Some(7_000));
        assert!(matches!(issuer.auth(), Auth::Mac(_)));
    }

    #[test]
    fn auth_uses_pop_until_a_mac_is_cached() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        assert!(matches!(issuer.auth(), Auth::Pop(_)));
        let status = status_for(issuer.account(), 100, Some(100));
        issuer.apply_status(status, Some(mac_ttl())).unwrap();
        assert!(matches!(issuer.auth(), Auth::Mac(_)));
    }

    #[cfg(feature = "vanilla")]
    #[test]
    fn clear_mac_falls_back_to_pop() {
        let mut issuer = Issuer::new(account(1), Duration::from_secs(60));
        let status = status_for(issuer.account(), 100, Some(100));
        issuer.apply_status(status, Some(mac_ttl())).unwrap();
        assert!(matches!(issuer.auth(), Auth::Mac(_)));
        issuer.clear_mac();
        assert!(matches!(issuer.auth(), Auth::Pop(_)));
    }
}
