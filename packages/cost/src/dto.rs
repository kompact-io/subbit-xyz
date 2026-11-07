//! Wire shapes only: what actually rides over the network (in
//! `subbit_l2_dto::Terms.catalog`, in x402's `SubbitExtra`, etc).
//! `crate::Catalog`, the compiled matchit-backed resolver, is a
//! separate, deliberately non-wire type built from `dto::Catalog` --
//! see its own module doc comment for why the two share a name
//! disambiguated only by this module path.

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

/// A breakdown of resources available from a provider: each entry
/// pairs a matchit path pattern with the full `ResourceInfo` it
/// resolves to. There is no separate "default" field -- a provider
/// wanting blanket coverage registers a catch-all entry of its own
/// (`"/{*rest}"`), at matchit's own lower priority, so specific routes
/// still win. Note matchit's own documented quirk: a catch-all never
/// matches the bare root path `/` itself (an empty capture), so a
/// provider that genuinely serves a priced resource at `/` needs a
/// second, literal `"/"` entry alongside the catch-all -- the two
/// coexist without conflict.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Catalog {
    #[n(0)]
    pub resources: Vec<(String, ResourceInfo)>,
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceInfo {
    #[n(0)]
    pub cost: Cost,
    #[n(1)]
    pub description: String,
    #[n(2)]
    pub mime_type: String,
    #[n(3)]
    pub max_timeout_seconds: u64,
    #[n(4)]
    pub service_name: Option<String>,
    #[n(5)]
    pub tags: Option<Vec<String>>,
    #[n(6)]
    pub icon_url: Option<String>,
}

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

    fn resource_info() -> ResourceInfo {
        ResourceInfo {
            cost: Cost::ByRequest(10),
            description: "Access to a metered resource".to_string(),
            mime_type: "application/octet-stream".to_string(),
            max_timeout_seconds: 60,
            service_name: None,
            tags: None,
            icon_url: None,
        }
    }

    #[test]
    fn catalog_roundtrips_with_no_resources() {
        let catalog = Catalog {
            resources: Vec::new(),
        };
        let bytes = minicbor::to_vec(&catalog).expect("encode failed");
        let decoded: Catalog = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(catalog, decoded);
    }

    #[test]
    fn catalog_roundtrips_with_a_resource() {
        let catalog = Catalog {
            resources: vec![("/special".to_string(), resource_info())],
        };
        let bytes = minicbor::to_vec(&catalog).expect("encode failed");
        let decoded: Catalog = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(catalog, decoded);
    }
}
