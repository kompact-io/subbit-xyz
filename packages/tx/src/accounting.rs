//! Byte-size accounting for a transaction being assembled
//! incrementally from staged [`Will`]s and [`Open`]s -- the thing
//! `tx-builder-proposal.md`'s "drain" step is meant to size a
//! `Batch` against, in place of a flat headcount cap
//! (`BuildConfig`'s `max_wills_per_tx`/`max_opens_per_tx`).
//!
//! Deliberately scoped to just the *marginal* cost of each included
//! item: [`Will`]/[`Open::size_estimate`], one [`step::redeemer_witness_overhead`]
//! per included `Will`, and one [`step::signer_bytes`] +
//! [`step::vkey_witness_bytes`] per *unique* signer across the whole
//! batch (named once in `required_signers`, signed once in the
//! witness set -- never per `Will`). It has no opinion on fuel,
//! collateral, change, or the fee itself -- those are assembly's
//! job, not this one's. A caller picks a `max_size` budget (protocol
//! max tx size, minus whatever margin it reserves for fuel/
//! collateral/change/fee) and hands it to [`Accounting::try_add_will`]/
//! [`Accounting::try_add_open`].

use std::collections::BTreeSet;

use subbit_core::Hash28;

use crate::{
    Open,
    step::{self, Will},
};

/// Running total: everything committed by `try_add_will`/
/// `try_add_open` so far. Cheap to clone -- every `try_add_*` clones
/// `self`, tries the addition on the clone, and only keeps it if
/// `size()` still fits.
#[derive(Debug, Clone, Default)]
pub struct Accounting {
    will_bytes: usize,
    n_wills: usize,
    open_bytes: usize,
    signers: BTreeSet<Hash28>,
}

impl Accounting {
    /// Exact marginal byte total for everything committed so far.
    pub fn size(&self) -> usize {
        self.will_bytes
            + self.n_wills * step::redeemer_witness_overhead()
            + self.open_bytes
            + self.signers.len() * (step::signer_bytes() + step::vkey_witness_bytes())
    }

    pub fn n_wills(&self) -> usize {
        self.n_wills
    }

    pub fn n_signers(&self) -> usize {
        self.signers.len()
    }

    /// Commits `will` (whose required signer is `signer`) only if
    /// doing so keeps `size()` at or under `max_size`; leaves `self`
    /// untouched and returns `false` otherwise.
    pub fn try_add_will(&mut self, will: &Will, signer: Hash28, max_size: usize) -> bool {
        let mut next = self.clone();
        next.will_bytes += will.size_estimate();
        next.n_wills += 1;
        next.signers.insert(signer);
        if next.size() > max_size {
            return false;
        }
        *self = next;
        true
    }

    /// Commits `open` only if doing so keeps `size()` at or under
    /// `max_size`.
    pub fn try_add_open(&mut self, open: &Open, max_size: usize) -> bool {
        let mut next = self.clone();
        next.open_bytes += open.size_estimate();
        if next.size() > max_size {
            return false;
        }
        *self = next;
        true
    }
}
