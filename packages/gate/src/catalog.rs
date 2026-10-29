//! Provider-side wrapping of `subbit-l2-dto`'s shared `CatalogIndex`:
//! adds `actual_cost`, settling a `ByEgress`/`ByClaim` quote against
//! the real response.

use http::HeaderMap;
use subbit_l2_dto::{CLAIM_COST_HEADER, Cost};

pub use subbit_l2_dto::{
    Catalog as CatalogDto, CatalogError as Error, CatalogIndex as Catalog, ResourceInfo,
    ResourceInfoOverride, normalize,
};

/// What `path` actually cost: `ByEgress` reads the real
/// `Content-Length` against `path`'s own rule; `ByClaim` reads the
/// provider's claimed `CLAIM_COST_HEADER` as-is. `None` when
/// nothing relevant is present, including for `ByRequest`;
/// `Ctx::refund_amount` treats `None` as nothing served, refunding
/// the full quote.
pub fn actual_cost(catalog: &Catalog, path: &str, headers: &HeaderMap) -> Option<u64> {
    match catalog.resolve(path).cost {
        Cost::ByEgress(rule) => {
            let len = parse_u64_header(headers, http::header::CONTENT_LENGTH.as_str())?;
            Some(rule.cost(len))
        }
        Cost::ByClaim(_) => parse_u64_header(headers, CLAIM_COST_HEADER),
        Cost::ByRequest(_) => None,
    }
}

fn parse_u64_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use subbit_l2_dto::CostByEgress;

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

    fn egress_catalog() -> Catalog {
        catalog(
            resource_info(Cost::ByEgress(CostByEgress {
                base: 5,
                per_kib: 2,
                max_bytes: 10_240,
            })),
            Vec::new(),
        )
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
            actual_cost(&catalog, "/resource", &content_length(1)),
            Some(7)
        );
        // exactly on a kib boundary: ceil(2048/1024) = 2
        assert_eq!(
            actual_cost(&catalog, "/resource", &content_length(2048)),
            Some(5 + 2 * 2)
        );
        // a zero-length response still owes the base
        assert_eq!(
            actual_cost(&catalog, "/resource", &content_length(0)),
            Some(5)
        );
    }

    #[test]
    fn egress_actual_cost_clamps_to_max_bytes() {
        let catalog = egress_catalog();
        assert_eq!(
            actual_cost(&catalog, "/resource", &content_length(1_000_000)),
            Some(catalog.cost("/resource"))
        );
    }

    #[test]
    fn egress_actual_cost_is_none_without_a_content_length_header() {
        let catalog = egress_catalog();
        assert_eq!(actual_cost(&catalog, "/resource", &HeaderMap::new()), None);
    }

    fn claim_catalog() -> Catalog {
        catalog(resource_info(Cost::ByClaim(42)), Vec::new())
    }

    #[test]
    fn claim_actual_cost_reads_the_claim_header() {
        let catalog = claim_catalog();
        let mut headers = HeaderMap::new();
        headers.insert(CLAIM_COST_HEADER, http::HeaderValue::from_static("7"));
        assert_eq!(actual_cost(&catalog, "/resource", &headers), Some(7));
        // no claim header at all: nothing to go on.
        assert_eq!(actual_cost(&catalog, "/resource", &HeaderMap::new()), None);
    }
}
