//! `dto::Catalog` compiled into a `matchit::Router` once, for repeated
//! lookups by path. Shared, identical logic for both sides of a
//! channel: a provider resolves this to decide what to charge, a
//! consumer resolves the exact same way to decide what it expects to
//! pay. What each side *does* about a path matching nothing here --
//! a provider's own blanket policy, a consumer seeking the caller's
//! consent -- is deliberately not this crate's concern: that's a
//! caller problem, not a shared trait or a generic parameter on this
//! type.

use matchit::Router;

use crate::dto::{Catalog as Dto, ResourceInfo};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("insert: {pattern}, {source:?}")]
    Insert {
        pattern: String,
        source: matchit::InsertError,
    },
}

pub struct Catalog {
    router: Router<ResourceInfo>,
}

impl Catalog {
    pub fn new(catalog: Dto) -> Result<Self, CatalogError> {
        let mut router = Router::new();
        for (pattern, info) in catalog.resources {
            router
                .insert(&pattern, info)
                .map_err(|source| CatalogError::Insert { pattern, source })?;
        }
        Ok(Self { router })
    }

    /// `path`'s resolved `ResourceInfo`, or `None` if nothing in the
    /// catalog matches -- the genuine "out of catalog" signal, not a
    /// silent default. A provider wanting blanket coverage registers
    /// its own catch-all entry (see `dto::Catalog`'s own doc comment).
    pub fn resolve(&self, path: &str) -> Option<&ResourceInfo> {
        self.router.at(path).ok().map(|m| m.value)
    }

    /// The quote for `path`: the most a request against it could
    /// ever cost, billed in full at spend time. `None` iff `resolve`
    /// is.
    pub fn cost(&self, path: &str) -> Option<u64> {
        self.resolve(path).map(|info| info.cost.quote())
    }

    /// Registers (or replaces) one pattern's entry. A mechanical
    /// primitive over `matchit::Router::insert` -- any policy for
    /// *when* to call this (e.g. persisting a consented price) is a
    /// caller concern, not this type's.
    pub fn insert(
        &mut self,
        pattern: impl Into<String>,
        info: ResourceInfo,
    ) -> Result<(), CatalogError> {
        let pattern = pattern.into();
        self.router
            .insert(&pattern, info)
            .map_err(|source| CatalogError::Insert { pattern, source })
    }

    /// The converse of `insert`: removes `pattern`'s entry, if one is
    /// registered under it, handing back whatever was there.
    pub fn remove(&mut self, pattern: impl Into<String>) -> Option<ResourceInfo> {
        self.router.remove(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{Cost, CostByEgress};

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

    fn resource_info_with(cost: Cost) -> ResourceInfo {
        ResourceInfo {
            cost,
            ..resource_info()
        }
    }

    fn index(resources: Vec<(String, ResourceInfo)>) -> Catalog {
        Catalog::new(Dto { resources }).unwrap()
    }

    #[test]
    fn resolve_is_none_when_nothing_matches() {
        let index = index(vec![("/special".to_string(), resource_info())]);
        assert!(index.resolve("/resource").is_none());
    }

    #[test]
    fn resolve_falls_back_to_a_registered_catch_all() {
        let index = index(vec![("/{*rest}".to_string(), resource_info())]);
        let info = index.resolve("/resource").expect("catch-all matches");
        assert_eq!(info.description, "Access to a metered resource");
        assert_eq!(info.mime_type, "application/octet-stream");
        assert_eq!(info.max_timeout_seconds, 60);
    }

    #[test]
    fn resolve_prefers_a_resource_specific_value_over_the_catch_all() {
        let index = index(vec![
            (
                "/{*rest}".to_string(),
                ResourceInfo {
                    mime_type: "application/json".to_string(),
                    ..resource_info()
                },
            ),
            (
                "/special".to_string(),
                ResourceInfo {
                    description: "A special resource".to_string(),
                    mime_type: "application/json".to_string(),
                    ..resource_info()
                },
            ),
        ]);

        let special = index.resolve("/special").expect("exact match");
        assert_eq!(special.description, "A special resource");
        assert_eq!(special.mime_type, "application/json");

        let other = index.resolve("/other").expect("catch-all matches");
        assert_eq!(other.description, "Access to a metered resource");
        assert_eq!(other.mime_type, "application/json");
    }

    #[test]
    fn by_request_cost_is_the_flat_amount() {
        let index = index(vec![(
            "/{*rest}".to_string(),
            resource_info_with(Cost::ByRequest(10)),
        )]);
        assert_eq!(index.cost("/resource"), Some(10));
    }

    #[test]
    fn by_egress_cost_is_the_quote_at_max_bytes() {
        let index = index(vec![(
            "/{*rest}".to_string(),
            resource_info_with(Cost::ByEgress(CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
        )]);
        assert_eq!(index.cost("/resource"), Some(25));
    }

    #[test]
    fn by_claim_cost_is_the_cap() {
        let index = index(vec![(
            "/{*rest}".to_string(),
            resource_info_with(Cost::ByClaim(42)),
        )]);
        assert_eq!(index.cost("/resource"), Some(42));
    }

    #[test]
    fn cost_rule_is_per_route_falling_back_to_the_catch_all() {
        let index = index(vec![
            (
                "/{*rest}".to_string(),
                resource_info_with(Cost::ByEgress(CostByEgress {
                    base: 5,
                    per_kib: 2,
                    max_bytes: 10_240,
                })),
            ),
            (
                "/special".to_string(),
                resource_info_with(Cost::ByEgress(CostByEgress {
                    base: 1,
                    per_kib: 1,
                    max_bytes: 4_096,
                })),
            ),
        ]);

        assert_eq!(index.cost("/special"), Some(5));
        assert_eq!(index.cost("/other"), Some(25));
    }

    #[test]
    fn insert_adds_a_new_pattern_at_runtime() {
        let mut index = index(Vec::new());
        assert!(index.resolve("/new").is_none());
        index.insert("/new", resource_info()).unwrap();
        assert!(index.resolve("/new").is_some());
    }

    #[test]
    fn remove_is_the_converse_of_insert() {
        let mut index = index(vec![("/special".to_string(), resource_info())]);
        assert!(index.resolve("/special").is_some());
        assert!(index.remove("/special").is_some());
        assert!(index.resolve("/special").is_none());
    }
}
