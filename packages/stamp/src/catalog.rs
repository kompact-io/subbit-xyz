//! The one `Catalog` `Stamp` consults before paying for a request: a
//! single, dialect-agnostic indexed lookup, owned by `Stamp` itself
//! (see `Stamp::set_catalog`/`Stamp::resolve_charge`) rather than
//! built in parallel by every caller that used to carry its own
//! `subbit_cost::Catalog` around (`local::Local`, each reqwest
//! middleware, `subbit-canary`'s `Consumer`). Loading it for the
//! first time and refreshing it later (e.g. vanilla re-fetching a
//! `.well-known/subbit` catalog after a provider changes its prices)
//! are the same call -- `set_catalog` just replaces whatever was
//! there.
//!
//! What to do when a path matches nothing in it is still entirely a
//! caller decision -- a [`MissPolicy`], supplied at runtime, never
//! guessed or defaulted here.

pub use subbit_cost::{Catalog, CatalogError};
use subbit_l2_dto::ResourceInfo;

/// What a resolved request actually costs to send, once any
/// [`MissPolicy`] has had its say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    /// Don't send the request at all.
    Reject,
    /// Send it with no payment envelope/header at all.
    Unpriced,
    /// Pay this amount.
    Priced(u64),
}

/// What to do about a path matching nothing in the catalog.
#[derive(Debug, Clone)]
pub enum MissDecision {
    /// Don't send the request.
    Reject,
    /// Send it unmetered, this one time.
    Unpriced,
    /// Pay `amount`, this one time -- not remembered.
    Once(u64),
    /// Pay `info.cost`'s quote, and register `pattern`/`info` in the
    /// catalog so every future match against it is a hit, not a miss.
    Learn(String, ResourceInfo),
}

/// Supplied at runtime by the caller: the only extension point this
/// module needs. A plain closure implements it directly
/// (`|_: &str| MissDecision::Reject` is a complete policy on its
/// own); anything stateful -- asking a human, remembering an answer
/// across calls -- is just a closure that captures that state
/// itself, not a separate type per policy shape.
pub trait MissPolicy: Send + Sync {
    fn on_miss(&self, path: &str) -> MissDecision;
}

impl<F> MissPolicy for F
where
    F: Fn(&str) -> MissDecision + Send + Sync,
{
    fn on_miss(&self, path: &str) -> MissDecision {
        self(path)
    }
}

/// Reject every miss outright -- the strictest policy, and the
/// previous hardcoded behavior of every caller in this crate.
pub fn reject(_path: &str) -> MissDecision {
    MissDecision::Reject
}

/// Let every miss through unmetered -- e.g. a provider trusted not to
/// charge for a path it never published a price for.
pub fn unpriced(_path: &str) -> MissDecision {
    MissDecision::Unpriced
}

/// Pay a fixed `amount` on every miss, never remembered.
pub fn fixed(amount: u64) -> impl Fn(&str) -> MissDecision + Send + Sync + Clone {
    move |_path: &str| MissDecision::Once(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_closure_is_a_complete_policy() {
        let policy: &dyn MissPolicy = &reject;
        assert!(matches!(policy.on_miss("/x"), MissDecision::Reject));
    }

    #[test]
    fn fixed_always_answers_the_same_amount() {
        let policy = fixed(42);
        assert!(matches!(policy.on_miss("/a"), MissDecision::Once(42)));
        assert!(matches!(policy.on_miss("/b"), MissDecision::Once(42)));
    }
}
