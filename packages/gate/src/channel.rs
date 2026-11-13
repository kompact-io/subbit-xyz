use minicbor::{Decode, Encode};
use redb::{TypeName, Value as RedbValue};
use serde::{Deserialize, Serialize};
use subbit_core::Iou;
use subbit_index_dto::OutputReference;

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
    /// No on-chain backing known for this channel (yet): nothing to cover
    /// any iou with. The wire's `NoChannel`.
    #[error("no backing")]
    NoBacking,
    /// The iou or spend exceeds what's backed/committed. The wire's own
    /// `InsufficientFunds`.
    #[error("insufficient funds")]
    InsufficientFunds,
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

    /// Whether a cumulative `committed` is covered by what's on chain: the
    /// part not already `subbed` must sit strictly inside `amount`.
    pub fn backs(&self, committed: u64) -> bool {
        committed.saturating_sub(self.subbed) < self.amount
    }
}

/// The mutable state of a channel, everything except its identity.
///
/// A channel is identified externally by a `Keytag` (`VerifyingKey` + `Tag`),
/// which is *not* stored here: it's the db key this value lives under.
/// Nothing on `Channel` needs it, signature verification (`verify_iou`) is
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

    /// TxO refs the index service's lineage report said are safe to
    /// settle (`subbit_gate::index_output`'s own doc) -- `None`
    /// means no `Output` has covered this channel yet, `Some(vec![])`
    /// means one has and found nothing safe to settle right now.
    #[n(5)]
    safe_to_settle: Option<Vec<OutputReference>>,
}

impl Channel {
    pub fn new(
        backing: Option<Backing>,
        iou: Option<Iou>,
        spent: u64,
        bucket: Bucket,
        aux: Aux,
        safe_to_settle: Option<Vec<OutputReference>>,
    ) -> Self {
        Self {
            backing,
            iou,
            spent,
            bucket,
            aux,
            safe_to_settle,
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

    pub fn safe_to_settle(&self) -> Option<&[OutputReference]> {
        self.safe_to_settle.as_deref()
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

    /// Replace the channel's `safe_to_settle` set.
    pub fn apply_safe_to_settle(&mut self, safe_to_settle: Option<Vec<OutputReference>>) {
        self.safe_to_settle = safe_to_settle;
    }

    /// Apply an already-verified `Iou`, replacing the current one if it's
    /// newer. Callers must have called `verify_iou` first, this doesn't
    /// check the signature itself.
    pub fn apply_iou(&mut self, iou: Iou) -> Result<u64, Error> {
        self.try_active()?;
        let curr = self.iou().map_or(0, |x| x.amount());
        let delta = iou.amount().saturating_sub(curr);
        if delta == 0 {
            return Err(Error::OldIou);
        }
        let Some(backing) = self.backing() else {
            return Err(Error::NoBacking);
        };
        if !backing.backs(iou.amount()) {
            return Err(Error::InsufficientFunds);
        }
        self.iou = Some(iou);
        Ok(delta)
    }

    /// Commit `cost` against the channel if it's backed.
    pub fn apply_spend(&mut self, cost: u64) -> Result<(), Error> {
        self.try_active()?;
        if !self.can_spend(cost) {
            return Err(Error::InsufficientFunds);
        }
        self.spent += cost;
        Ok(())
    }

    /// Refund `amount` off the running spent total, floored at zero.
    pub fn apply_refund(&mut self, amount: u64) {
        self.spent = self.spent.saturating_sub(amount);
    }

    pub fn summary(&self) -> ChannelSummary {
        ChannelSummary {
            is_active: self.aux().is_active(),
            iou: self.iou().cloned(),
            safe_to_settle: self.safe_to_settle().map(<[_]>::to_vec).unwrap_or_default(),
            backing: self.backing().cloned(),
        }
    }
}

/// A channel's state as seen by an external actor deciding whether to
/// act on it (e.g. a provider tx service deciding to `Sub`/`Settle`) --
/// just enough to decide, not the full `Channel`.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct ChannelSummary {
    #[n(0)]
    pub is_active: bool,
    #[n(1)]
    pub iou: Option<Iou>,
    /// Flattened from `Channel::safe_to_settle`'s `Option<Vec<_>>`:
    /// an external reader has no use for "no `Output` has covered
    /// this channel yet" versus "one has, and found nothing" -- both
    /// mean the same thing to it, nothing known safe to settle.
    #[n(2)]
    pub safe_to_settle: Vec<OutputReference>,
    /// `None` here means the index's lineage found no open line to
    /// back this channel with -- a `Closed` on-chain UTxO is then
    /// uncontested (nothing else is claiming to be the live backing),
    /// so it's as safe to settle as if its own `txo_ref` were in
    /// `safe_to_settle` directly. See `subbit_claim::decide`'s own use
    /// of this.
    #[n(3)]
    pub backing: Option<Backing>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn iou(amount: u64) -> Iou {
        Iou::new(amount, subbit_core::Signature::from([0u8; 64]))
    }

    fn channel(backing: Option<Backing>) -> Channel {
        Channel::new(
            backing,
            None,
            0,
            Default::default(),
            Default::default(),
            None,
        )
    }

    #[test]
    fn apply_iou_rejects_without_a_backing() {
        let mut channel = channel(None);
        assert!(matches!(channel.apply_iou(iou(1)), Err(Error::NoBacking)));
        assert!(channel.iou().is_none());
    }

    #[test]
    fn apply_iou_accepts_up_to_the_backed_amount_and_no_further() {
        // 1_000 on chain of which 200 already subbed: an iou may commit
        // strictly less than 1_200 cumulative (`Backing::backs`).
        let mut channel = channel(Some(Backing::new(1_000, 200)));
        assert_eq!(channel.apply_iou(iou(1_199)).unwrap(), 1_199);
        assert!(matches!(
            channel.apply_iou(iou(1_200)),
            Err(Error::InsufficientFunds)
        ));
        // The rejected iou is not remembered.
        assert_eq!(channel.iou().map(|x| x.amount()), Some(1_199));
    }

    #[test]
    fn apply_iou_still_rejects_a_non_increasing_amount_first() {
        let mut channel = channel(Some(Backing::new(1_000, 0)));
        channel.apply_iou(iou(500)).unwrap();
        assert!(matches!(channel.apply_iou(iou(500)), Err(Error::OldIou)));
    }
}
