use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use subbit_core::{Duration, Hash28};

use crate::{Asset, Catalog, OutputRef};

/// Provider terms of service.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Terms {
    /// Which Cardano network this provider is deployed on.
    #[n(0)]
    pub network: crate::Network,
    /// The subbit validator's own script hash. Consumer/client should
    /// ensure this is acceptable before engaging.
    #[n(1)]
    pub script: Hash28,
    /// Required asset, as appears in `datum.constants.asset`.
    #[n(2)]
    pub asset: Asset,
    /// Required provider key, as appears in `datum.constants.provider`.
    #[n(3)]
    pub provider: Hash28,
    /// Close period, as appears in `datum.constants.close_period`.
    #[n(4)]
    pub close_period: Duration,
    /// Block depth a UTxO must reach before the provider treats it as
    /// settled.
    #[n(5)]
    pub confirmation_depth: u32,
    /// The minimum amount the first IOU must carry when opening a
    /// channel. It becomes part of the consumer's spendable balance.
    /// Prevents dust accounts.
    #[n(6)]
    pub min_iou_amount: u64,
    /// Resource catalog. Every script/asset must define its own
    /// catalog. Consumer should consent to the catalog and its cost
    /// model prior to autonomous engagement; the issuer disengages a
    /// server that deviates from it, to the consumer's detriment.
    #[n(7)]
    pub catalog: Catalog,
    /// Tx helper: reference script for the validator.
    #[n(8)]
    pub reference_script: Option<OutputRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_roundtrips() {
        let terms = Terms {
            network: crate::Network::Preprod,
            script: Hash28::from([1u8; 28]),
            asset: Asset::Lovelace,
            provider: Hash28::from([2u8; 28]),
            close_period: Duration::from_secs(900),
            confirmation_depth: 3,
            min_iou_amount: 500,
            catalog: Catalog {
                default: crate::ResourceInfo {
                    cost: crate::Cost::ByRequest(1_000),
                    description: "Access to a metered resource".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    max_timeout_seconds: 60,
                    service_name: None,
                    tags: None,
                    icon_url: None,
                },
                resources: Vec::new(),
            },
            reference_script: Some(OutputRef {
                tx_hash: subbit_core::Hash32::from([3u8; 32]),
                index: 0,
            }),
        };
        let bytes = minicbor::to_vec(&terms).expect("encode failed");
        let decoded: Terms = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(terms, decoded);
    }
}
