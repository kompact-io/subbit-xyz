//! Given a request path, resolve what a provider has configured for
//! it: cost, and presentation fields (x402's extra ones), together,
//! falling back to a provider-wide default. "Routing" isn't part of
//! this vocabulary: path-pattern matching is an implementation detail
//! (`matchit`), not a concept callers name.

use http::HeaderMap;
use matchit::Router;
use subbit_l2_dto::CLAIM_COST_HEADER;

pub use subbit_l2_dto::{
    Catalog as CatalogDto, Cost, CostByEgress, ResourceInfo, ResourceInfoOverride,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("insert: {pattern}, {source:?}")]
    Insert {
        pattern: String,
        source: matchit::InsertError,
    },
}

/// The runtime resolver over a `subbit_l2_dto::Catalog`: the
/// provider's own config, plus the compiled `matchit::Router` built
/// from it once, rather than on every request.
pub struct Catalog {
    default: ResourceInfo,
    router: Router<ResourceInfoOverride>,
}

impl Catalog {
    pub fn new(catalog: CatalogDto) -> Result<Self, Error> {
        let mut router = Router::new();
        for (pattern, over) in catalog.resources {
            router
                .insert(&pattern, over)
                .map_err(|source| Error::Insert { pattern, source })?;
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

    /// The quote: the most a request against `path` could ever cost,
    /// billed in full at spend time. For `ByEgress`/`ByClaim` this is
    /// a ceiling, not what the request will actually be charged; see
    /// `actual_cost`.
    pub fn cost(&self, path: &str) -> u64 {
        match self.resolve(path).cost {
            Cost::ByRequest(amount) => amount,
            Cost::ByEgress(rule) => egress_cost(rule.base, rule.per_kib, rule.max_bytes),
            Cost::ByClaim(amount) => amount,
        }
    }

    /// What this request actually cost: `ByEgress` reads the real
    /// `Content-Length` against `path`'s own rule; `ByClaim` reads the
    /// provider's claimed `CLAIM_COST_HEADER` as-is. `None` when
    /// nothing relevant is present, including for `ByRequest`;
    /// `Ctx::refund_amount` treats `None` as nothing served, refunding
    /// the full quote.
    pub fn actual_cost(&self, path: &str, headers: &HeaderMap) -> Option<u64> {
        match self.resolve(path).cost {
            Cost::ByEgress(rule) => {
                let len = parse_u64_header(headers, http::header::CONTENT_LENGTH.as_str())?;
                Some(egress_cost(
                    rule.base,
                    rule.per_kib,
                    len.min(rule.max_bytes),
                ))
            }
            Cost::ByClaim(_) => parse_u64_header(headers, CLAIM_COST_HEADER),
            Cost::ByRequest(_) => None,
        }
    }

    pub fn info(&self, path: &str) -> ResourceInfo {
        self.resolve(path)
    }
}

/// `matchit` patterns are always leading-slash anchored (`"/resource"`);
/// axum's wildcard route capture strips that slash before handlers see
/// it, so every path reaching a `Catalog` method goes through here
/// rather than each caller re-adding it.
pub(crate) fn normalize(path: &str) -> std::borrow::Cow<'_, str> {
    if path.starts_with('/') {
        std::borrow::Cow::Borrowed(path)
    } else {
        std::borrow::Cow::Owned(format!("/{path}"))
    }
}

fn egress_cost(base: u64, per_kib: u64, bytes: u64) -> u64 {
    base + per_kib * bytes.div_ceil(1024)
}

fn parse_u64_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource_info(cost: Cost) -> ResourceInfo {
        ResourceInfo {
            cost,
            description: "Access to a metered resource".to_string(),
            mime_type: "application/octet-stream".to_string(),
            max_timeout_seconds: 60,
            service_name: None,
            tags: None,
            icon_url: None,
        }
    }

    fn catalog(default: ResourceInfo, resources: Vec<(String, ResourceInfoOverride)>) -> Catalog {
        Catalog::new(CatalogDto { default, resources }).unwrap()
    }

    #[test]
    fn info_falls_back_to_the_default() {
        let catalog = catalog(resource_info(Cost::ByRequest(10)), Vec::new());
        let info = catalog.info("/resource");
        assert_eq!(info.description, "Access to a metered resource");
        assert_eq!(info.mime_type, "application/octet-stream");
        assert_eq!(info.max_timeout_seconds, 60);
    }

    #[test]
    fn info_prefers_a_resource_specific_value_over_the_default() {
        let catalog = catalog(
            ResourceInfo {
                mime_type: "application/json".to_string(),
                ..resource_info(Cost::ByRequest(10))
            },
            vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    description: Some("A special resource".to_string()),
                    ..Default::default()
                },
            )],
        );

        let special = catalog.info("/special");
        assert_eq!(special.description, "A special resource");
        assert_eq!(special.mime_type, "application/json");

        let other = catalog.info("/other");
        assert_eq!(other.description, "Access to a metered resource");
        assert_eq!(other.mime_type, "application/json");
    }

    fn egress_catalog() -> Catalog {
        catalog(
            resource_info(Cost::ByEgress(subbit_l2_dto::CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
            Vec::new(),
        )
    }

    #[test]
    fn egress_quote_is_the_formula_at_max_bytes() {
        let catalog = egress_catalog();
        // base 5 + per_kib 2 * ceil(10_240 / 1024) = 5 + 2*10 = 25
        assert_eq!(catalog.cost("/resource"), 25);
    }

    fn content_length(bytes: u64) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&bytes.to_string()).unwrap(),
        );
        headers
    }

    #[test]
    fn egress_actual_cost_uses_the_real_length_ceil_rounded() {
        let catalog = egress_catalog();
        // base 5 + per_kib 2 * ceil(1 / 1024) = 5 + 2*1 = 7
        assert_eq!(
            catalog.actual_cost("/resource", &content_length(1)),
            Some(7)
        );
        // exactly on a kib boundary: ceil(2048/1024) = 2
        assert_eq!(
            catalog.actual_cost("/resource", &content_length(2048)),
            Some(5 + 2 * 2)
        );
        // a zero-length response still owes the base
        assert_eq!(
            catalog.actual_cost("/resource", &content_length(0)),
            Some(5)
        );
    }

    #[test]
    fn egress_actual_cost_clamps_to_max_bytes() {
        let catalog = egress_catalog();
        assert_eq!(
            catalog.actual_cost("/resource", &content_length(1_000_000)),
            Some(catalog.cost("/resource"))
        );
    }

    #[test]
    fn egress_actual_cost_is_none_without_a_content_length_header() {
        let catalog = egress_catalog();
        assert_eq!(catalog.actual_cost("/resource", &HeaderMap::new()), None);
    }

    #[test]
    fn egress_rule_is_per_route_falling_back_to_the_default() {
        let catalog = catalog(
            resource_info(Cost::ByEgress(subbit_l2_dto::CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
            vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    cost: Some(Cost::ByEgress(subbit_l2_dto::CostByEgress {
                        base: 1,
                        per_kib: 1,
                        max_bytes: 4_096,
                    })),
                    ..Default::default()
                },
            )],
        );

        // /special: base 1 + per_kib 1 * ceil(4096/1024) = 1 + 1*4 = 5
        assert_eq!(catalog.cost("/special"), 5);
        // /other falls back to the default: base 5 + per_kib 2 * 10 = 25
        assert_eq!(catalog.cost("/other"), 25);
    }

    fn claim_catalog() -> Catalog {
        catalog(resource_info(Cost::ByClaim(42)), Vec::new())
    }

    #[test]
    fn claim_quote_is_the_default() {
        let catalog = claim_catalog();
        assert_eq!(catalog.cost("/resource"), 42);

        let mut headers = HeaderMap::new();
        headers.insert(CLAIM_COST_HEADER, http::HeaderValue::from_static("7"));
        assert_eq!(catalog.actual_cost("/resource", &headers), Some(7));
        // no claim header at all: nothing to go on.
        assert_eq!(catalog.actual_cost("/resource", &HeaderMap::new()), None);
    }

    #[test]
    fn claim_rule_is_per_route_falling_back_to_the_default() {
        let catalog = catalog(
            resource_info(Cost::ByClaim(42)),
            vec![(
                "/special".to_string(),
                ResourceInfoOverride {
                    cost: Some(Cost::ByClaim(10)),
                    ..Default::default()
                },
            )],
        );

        assert_eq!(catalog.cost("/special"), 10);
        assert_eq!(catalog.cost("/other"), 42);
    }
}
