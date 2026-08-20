use minicbor::{Decode, Encode};
use redb::{TypeName, Value as RedbValue};
use serde::{Deserialize, Serialize};
use subbit_core::Iou;

use crate::{Aux, Bucket};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("other")]
    Other,
    #[error("not active")]
    Inactive,
    #[error("iou is old")]
    OldIou,
    #[error("iou invalid")]
    InvalidIou,
    #[error("cost exceeds what is spendable (deactived, insufficient funds, _etc_)")]
    Unspendable,
}

impl From<Error> for subbit_core::envelope::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Other => Self::Other,
            Error::Inactive => Self::Inactive,
            Error::OldIou => Self::OldIou,
            Error::InvalidIou => Self::InvalidIou,
            Error::Unspendable => Self::Unspendable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct Backing {
    #[n(0)]
    amount: u64,
    #[n(1)]
    subbed: u64,
}

impl Backing {
    pub fn new(amount: u64, subbed: u64) -> Self {
        Self { amount, subbed }
    }

    pub fn backs(&self, committed: u64) -> bool {
        committed - self.subbed < self.amount
    }
}

/// The mutable state of a channel — everything except its identity.
///
/// A channel is identified externally by a `Keytag` (`VerifyingKey` + `Tag`),
/// which is *not* stored here: it's the db key this value lives under.
/// Nothing on `Channel` needs it — signature verification (`verify_iou`) is
/// the caller's job, done before `apply_iou` is called.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct Channel {
    #[n(0)]
    backing: Option<Backing>,

    #[n(1)]
    iou: Option<Iou>,

    #[n(2)]
    spent: u64,

    #[n(3)]
    bucket: Bucket,

    #[n(4)]
    aux: Aux,
}

impl Channel {
    pub fn new(
        backing: Option<Backing>,
        iou: Option<Iou>,
        spent: u64,
        bucket: Bucket,
        aux: Aux,
    ) -> Self {
        Self {
            backing,
            iou,
            spent,
            bucket,
            aux,
        }
    }

    pub fn iou(&self) -> Option<&Iou> {
        self.iou.as_ref()
    }

    pub fn bucket(&self) -> &Bucket {
        &self.bucket
    }

    pub fn backing(&self) -> Option<&Backing> {
        self.backing.as_ref()
    }

    pub fn spent(&self) -> u64 {
        self.spent
    }

    pub fn aux(&self) -> &Aux {
        &self.aux
    }

    pub fn committed(&self) -> Option<u64> {
        self.iou().map(|x| x.amount())
    }

    pub fn subbed(&self) -> Option<u64> {
        self.backing().map(|x| x.subbed)
    }

    pub fn uncommitted(&self) -> Option<u64> {
        if let Some(backing) = self.backing()
            && let Some(committed) = self.committed()
        {
            Some(
                backing
                    .amount
                    .saturating_sub(committed.saturating_sub(backing.subbed)),
            )
        } else {
            None
        }
    }

    pub fn try_active(&self) -> Result<(), Error> {
        if self.aux().is_active() {
            Ok(())
        } else {
            Err(Error::Inactive)
        }
    }

    pub fn spendable(&self) -> Option<u64> {
        if self.aux().is_active() {
            self.committed().map(|x| x.saturating_sub(self.spent()))
        } else {
            None
        }
    }

    pub fn can_spend(&self, cost: u64) -> bool {
        self.spendable().is_some_and(|spendable| spendable >= cost)
    }

    /// Replace/top-up the channel's backing.
    pub fn apply_backing(&mut self, backing: Option<Backing>) {
        self.backing = backing;
    }

    /// Apply an already-verified `Iou`, replacing the current one if it's
    /// newer. Callers must have called `verify_iou` first — this doesn't
    /// check the signature itself.
    pub fn apply_iou(&mut self, iou: Iou) -> Result<u64, Error> {
        self.try_active()?;
        let curr = self.iou().map_or(0, |x| x.amount());
        let delta = iou.amount().saturating_sub(curr);
        if delta == 0 {
            return Err(Error::OldIou);
        }
        self.iou = Some(iou);
        Ok(delta)
    }

    /// Commit `cost` against the channel if it's backed.
    pub fn apply_spend(&mut self, cost: u64) -> Result<(), Error> {
        self.try_active()?;
        if !self.can_spend(cost) {
            return Err(Error::Unspendable);
        }
        self.spent += cost;
        Ok(())
    }

    /// Refund `amount` off the running spent total, floored at zero.
    pub fn apply_refund(&mut self, amount: u64) {
        self.spent = self.spent.saturating_sub(amount);
    }
}

// ---------------------------------------------------------------------------
// redb
// ---------------------------------------------------------------------------

impl RedbValue for Channel {
    type SelfType<'a> = Channel;
    type AsBytes<'a> = Vec<u8>;
    fn fixed_width() -> Option<usize> {
        None
    }
    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        // NOTE: infallible by redb's trait contract.
        postcard::from_bytes(data).expect("server::channel: invalid postcard for Channel")
    }
    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        postcard::to_allocvec(value).expect("server::channel: postcard encode failed")
    }
    fn type_name() -> TypeName {
        TypeName::new("server::Channel")
    }
}
