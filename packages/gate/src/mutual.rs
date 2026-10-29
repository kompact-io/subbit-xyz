//! Cooperative close: co-sign and broadcast a transaction the consumer
//! already built, signed, and sent. Not `subbit-tx`, nothing here
//! constructs a transaction, the consumer did that; this is parsing,
//! checking, signing, and submitting a transaction that already
//! exists.

use crate::Keytag;

/// What cooperatively closing a channel needs: given the consumer's
/// already-built close transaction, an implementation must verify it
/// (a single first output paying the provider's unredeemed share, no
/// stake, no datum, no script; the channel's keytag as an input; and,
/// the one correctness property this exists to enforce, **none** of
/// its inputs belonging to the provider's own wallet), then co-sign
/// and broadcast it.
pub trait MutualService: Send + Sync {
    fn close(&self, keytag: &Keytag, transaction: &[u8]) -> Result<(), MutualError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MutualError {
    #[error("mutual close not supported")]
    NotSupported,
}

/// Placeholder `MutualService`: nothing implements cooperative close
/// yet, so this always errors. `Ctx::from_config` uses this until a
/// real implementation exists to pass to `Ctx::new` instead.
#[derive(Debug, Default)]
pub struct Unsupported;

impl MutualService for Unsupported {
    fn close(&self, _keytag: &Keytag, _transaction: &[u8]) -> Result<(), MutualError> {
        Err(MutualError::NotSupported)
    }
}
