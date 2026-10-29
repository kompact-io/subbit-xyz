use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Consumer should a priori consent to a cost (model). Their client should
/// keep their own accounting to ensure any provider misbehaving is
/// immediately disengaged.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Cost {
    /// A flat fee per request.
    #[n(0)]
    ByRequest(#[n(0)] u64),
    /// An affine fee dependent on content length. Client must pay
    /// _max_ upfront, but should be "refunded" the balance unspent,
    /// calculated a posteriori.
    #[n(1)]
    ByEgress(#[n(0)] CostByEgress),
    /// Capped at this amount, but the provider's precise cost metric
    /// is unverifiable by the consumer (e.g. AI billing for token
    /// count with "internal tokens"). Consumer will need an
    /// independent assessment of whether the provider is behaving.
    #[n(2)]
    ByClaim(#[n(0)] u64),
}

/// `ByEgress`'s own formula: a fixed `base` plus `per_kib` for every
/// (ceil-rounded) kibibyte, capped at `max_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CostByEgress {
    #[n(0)]
    pub base: u64,
    #[n(1)]
    pub per_kib: u64,
    #[n(2)]
    pub max_bytes: u64,
}

impl CostByEgress {
    /// The charge for `bytes` of content, clamped to `max_bytes`
    /// before applying `base`/`per_kib`.
    pub fn cost(&self, bytes: u64) -> u64 {
        self.base + self.per_kib * bytes.min(self.max_bytes).div_ceil(1024)
    }
}

impl Cost {
    /// The most a request under this cost could ever be charged,
    /// billed in full upfront.
    pub fn quote(&self) -> u64 {
        match self {
            Cost::ByRequest(amount) => *amount,
            Cost::ByEgress(rule) => rule.cost(rule.max_bytes),
            Cost::ByClaim(amount) => *amount,
        }
    }
}

/// `ByClaim`'s own header names, application to proxy to
/// `subbit-gate` and back, unchanged at every hop: fixed, not
/// provider-configurable, so every hop agrees on them without being
/// told.
/// TODO :: This is not for clients; its for the gated application.
pub const CLAIM_BUDGET_HEADER: &str = "SUBBIT_BUDGET";
pub const CLAIM_COST_HEADER: &str = "SUBBIT_COST";

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(cost: &Cost) -> Cost {
        let bytes = minicbor::to_vec(cost).expect("encode failed");
        minicbor::decode(&bytes).expect("decode failed")
    }

    #[test]
    fn by_request_roundtrips() {
        let cost = Cost::ByRequest(7);
        assert_eq!(cost, roundtrip(&cost));
    }

    #[test]
    fn by_egress_roundtrips() {
        let cost = Cost::ByEgress(CostByEgress {
            base: 5,
            per_kib: 2,
            max_bytes: 1_048_576,
        });
        assert_eq!(cost, roundtrip(&cost));
    }

    #[test]
    fn by_claim_roundtrips() {
        let cost = Cost::ByClaim(1_000);
        assert_eq!(cost, roundtrip(&cost));
    }

    #[test]
    fn by_request_quote_is_the_flat_amount() {
        assert_eq!(Cost::ByRequest(10).quote(), 10);
    }

    #[test]
    fn by_egress_quote_is_the_formula_at_max_bytes() {
        let cost = Cost::ByEgress(CostByEgress {
            base: 5,
            per_kib: 2,
            max_bytes: 10_240,
        });
        assert_eq!(cost.quote(), 25);
    }

    #[test]
    fn by_claim_quote_is_the_cap() {
        assert_eq!(Cost::ByClaim(42).quote(), 42);
    }
}
