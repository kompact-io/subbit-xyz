use matchit::Router;
use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::Cost;

// A breakdown of resources available from provider.
// Includes the the cost (model) and info fields inspired x402's resource info.
// Find resource info by URL on first matchit match of `Catalog.resource[].0`.
// If no match or relevant field is None, then falls back to default field value.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Catalog {
    #[n(0)]
    pub default: ResourceInfo,
    #[n(1)]
    pub resources: Vec<(String, ResourceInfoOverride)>,
}

// Default requires fields specified.
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

/// A per-resource override of `Catalog::default`; any field left
/// `None` falls through to it.
#[derive(Debug, Clone, Default, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceInfoOverride {
    #[n(0)]
    pub cost: Option<Cost>,
    #[n(1)]
    pub description: Option<String>,
    #[n(2)]
    pub mime_type: Option<String>,
    #[n(3)]
    pub max_timeout_seconds: Option<u64>,
    #[n(4)]
    pub service_name: Option<String>,
    #[n(5)]
    pub tags: Option<Vec<String>>,
    #[n(6)]
    pub icon_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("insert: {pattern}, {source:?}")]
    Insert {
        pattern: String,
        source: matchit::InsertError,
    },
}

/// `Catalog` compiled into a `matchit::Router` once, for repeated
/// lookups by path.
/// This is more logic than I'd like to have in a dto crate,
/// but it is shared logic.
pub struct CatalogIndex {
    default: ResourceInfo,
    router: Router<ResourceInfoOverride>,
}

impl CatalogIndex {
    pub fn new(catalog: Catalog) -> Result<Self, CatalogError> {
        let mut router = Router::new();
        for (pattern, over) in catalog.resources {
            router
                .insert(&pattern, over)
                .map_err(|source| CatalogError::Insert { pattern, source })?;
        }
        Ok(Self {
            default: catalog.default,
            router,
        })
    }

    /// `path`'s fully resolved `ResourceInfo`: its own override, with
    /// any field it didn't set falling through to `default`.
    pub fn resolve(&self, path: &str) -> ResourceInfo {
        let path = normalize(path);
        let over = self.router.at(&path).ok().map(|m| m.value);
        ResourceInfo {
            cost: over
                .and_then(|o| o.cost.clone())
                .unwrap_or_else(|| self.default.cost.clone()),
            description: over
                .and_then(|o| o.description.clone())
                .unwrap_or_else(|| self.default.description.clone()),
            mime_type: over
                .and_then(|o| o.mime_type.clone())
                .unwrap_or_else(|| self.default.mime_type.clone()),
            max_timeout_seconds: over
                .and_then(|o| o.max_timeout_seconds)
                .unwrap_or(self.default.max_timeout_seconds),
            service_name: over
                .and_then(|o| o.service_name.clone())
                .or_else(|| self.default.service_name.clone()),
            tags: over
                .and_then(|o| o.tags.clone())
                .or_else(|| self.default.tags.clone()),
            icon_url: over
                .and_then(|o| o.icon_url.clone())
                .or_else(|| self.default.icon_url.clone()),
        }
    }

    /// The quote for `path`: the most a request against it could
    /// ever cost, billed in full at spend time.
    pub fn cost(&self, path: &str) -> u64 {
        self.resolve(path).cost.quote()
    }
}

/// `matchit` patterns are always leading-slash anchored (`"/resource"`).
pub fn normalize(path: &str) -> std::borrow::Cow<'_, str> {
    if path.starts_with('/') {
        std::borrow::Cow::Borrowed(path)
    } else {
        std::borrow::Cow::Owned(format!("/{path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CostByEgress;

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
    fn catalog_roundtrips_with_no_overrides() {
        let catalog = Catalog {
            default: resource_info(),
            resources: Vec::new(),
        };
        let bytes = minicbor::to_vec(&catalog).expect("encode failed");
        let decoded: Catalog = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(catalog, decoded);
    }

    #[test]
    fn catalog_roundtrips_with_a_partial_override() {
        let catalog = Catalog {
            default: resource_info(),
            resources: vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    description: Some("A special resource".to_string()),
                    ..Default::default()
                },
            )],
        };
        let bytes = minicbor::to_vec(&catalog).expect("encode failed");
        let decoded: Catalog = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(catalog, decoded);
    }

    fn resource_info_with(cost: Cost) -> ResourceInfo {
        ResourceInfo {
            cost,
            ..resource_info()
        }
    }

    fn index(
        default: ResourceInfo,
        resources: Vec<(String, ResourceInfoOverride)>,
    ) -> CatalogIndex {
        CatalogIndex::new(Catalog { default, resources }).unwrap()
    }

    #[test]
    fn resolve_falls_back_to_the_default() {
        let index = index(resource_info(), Vec::new());
        let info = index.resolve("/resource");
        assert_eq!(info.description, "Access to a metered resource");
        assert_eq!(info.mime_type, "application/octet-stream");
        assert_eq!(info.max_timeout_seconds, 60);
    }

    #[test]
    fn resolve_prefers_a_resource_specific_value_over_the_default() {
        let index = index(
            ResourceInfo {
                mime_type: "application/json".to_string(),
                ..resource_info()
            },
            vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    description: Some("A special resource".to_string()),
                    ..Default::default()
                },
            )],
        );

        let special = index.resolve("/special");
        assert_eq!(special.description, "A special resource");
        assert_eq!(special.mime_type, "application/json");

        let other = index.resolve("/other");
        assert_eq!(other.description, "Access to a metered resource");
        assert_eq!(other.mime_type, "application/json");
    }

    #[test]
    fn by_request_cost_is_the_flat_amount() {
        let index = index(resource_info_with(Cost::ByRequest(10)), Vec::new());
        assert_eq!(index.cost("/resource"), 10);
    }

    #[test]
    fn by_egress_cost_is_the_quote_at_max_bytes() {
        let index = index(
            resource_info_with(Cost::ByEgress(CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
            Vec::new(),
        );
        assert_eq!(index.cost("/resource"), 25);
    }

    #[test]
    fn by_claim_cost_is_the_cap() {
        let index = index(resource_info_with(Cost::ByClaim(42)), Vec::new());
        assert_eq!(index.cost("/resource"), 42);
    }

    #[test]
    fn cost_rule_is_per_route_falling_back_to_the_default() {
        let index = index(
            resource_info_with(Cost::ByEgress(CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
            vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    cost: Some(Cost::ByEgress(CostByEgress {
                        base: 1,
                        per_kib: 1,
                        max_bytes: 4_096,
                    })),
                    ..Default::default()
                },
            )],
        );

        assert_eq!(index.cost("/special"), 5);
        assert_eq!(index.cost("/other"), 25);
    }
}
