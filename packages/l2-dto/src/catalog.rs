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

#[cfg(test)]
mod tests {
    use super::*;

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
}
