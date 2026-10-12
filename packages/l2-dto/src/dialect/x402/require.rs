//! `scheme`/`network`/`amount`/`pay_to`/`max_timeout_seconds`/`asset`
//! are the same shape for every x402 scheme; only `extra` varies.
//! That means a foreign scheme's entry in `accepts[]` already parses
//! fine as a [`SubbitAccept`] (its `extra` just lands in
//! [`SubbitExtra::Unknown`]), so no separate tolerant sum type is
//! needed at the whole-entry level.

use serde::{Deserialize, Serialize};
use subbit_core::Hash28;

use super::spec;
use crate::{ChannelStatus, Terms};

pub const SCHEME: &str = "batch-settlement";

/// The 402 signal, `accepts` read as [`SubbitAccept`]. Sent twice,
/// same shape: as the initial challenge, and resent corrective when
/// a `voucher`/`deposit`'s cumulative amount didn't match the
/// provider's record.
pub type Require = spec::PaymentRequired<SubbitExtra>;

/// One `accepts[]` entry. `scheme`/`pay_to` are always derived,
/// enforced by `new`.
pub type SubbitAccept = spec::PaymentRequirements<SubbitExtra>;

impl SubbitAccept {
    /// Everything here but `amount`/`max_timeout_seconds` (the
    /// quote's own) and `channel_status` (the channel's current,
    /// dynamic state) comes straight off `terms`.
    pub fn new(
        terms: &Terms,
        amount: String,
        max_timeout_seconds: u64,
        channel_status: Option<ChannelStatus>,
    ) -> Self {
        Self {
            scheme: SCHEME.to_string(),
            network: terms.network.to_caip2(),
            amount,
            pay_to: terms.script.to_string(),
            max_timeout_seconds,
            asset: asset_string(&terms.asset),
            extra: SubbitExtra::subbit_v1(terms, channel_status),
        }
    }
}

/// x402's own `asset` field wants "token contract address, or ISO
/// 4217 currency code for fiat" (its spec's own wording), not a
/// CAIP-19 identifier (`Asset::to_caip19`): this is that simpler,
/// x402-specific convention instead.
fn asset_string(asset: &crate::Asset) -> String {
    match asset {
        crate::Asset::Lovelace => "lovelace".to_string(),
        crate::Asset::Native { hash, name } => format!("{hash}.{}", hex::encode(name)),
    }
}

/// Subbit's own `extra`. `Known`'s own `#[serde(tag = "kind")]`
/// fails to deserialize on a missing or unrecognized `kind` (serde's
/// ordinary internal-tagging behavior); since that happens inside
/// this enum's own `#[serde(untagged)]` trial, the failure is caught
/// here rather than propagating, and `Unknown` is tried next.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubbitExtra {
    Known(KnownExtra),
    Unknown(serde_json::Value),
}

/// Recognized `kind`s. One today; a future second kind (e.g. a
/// sponsored-channel variant) is just another variant here, `Known`
/// is where the door was left open.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum KnownExtra {
    #[serde(rename = "subbit-v1")]
    SubbitV1 {
        // `Terms::script`/`::asset` are already covered by
        // `SubbitAccept`'s own `pay_to`/`asset`, so only the rest of
        // `Terms`'s channel-open fields are inlined here.
        provider: Hash28,
        close_period: subbit_core::Duration,
        confirmation_depth: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        reference_script: Option<crate::OutputRef>,
        /// The minimum amount the first IOU must carry when opening a
        /// channel. Provider-wide, not per-script, set once during
        /// the initial handshake.
        min_iou_amount: u64,
        /// Present only on a corrective `Require`: the one fact here
        /// that varies per consumer.
        #[serde(skip_serializing_if = "Option::is_none")]
        channel_status: Option<ChannelStatus>,
    },
}

impl SubbitExtra {
    pub fn subbit_v1(terms: &Terms, channel_status: Option<ChannelStatus>) -> Self {
        Self::Known(KnownExtra::SubbitV1 {
            provider: terms.provider,
            close_period: terms.close_period,
            confirmation_depth: terms.confirmation_depth,
            reference_script: terms.reference_script,
            min_iou_amount: terms.min_iou_amount,
            channel_status,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn terms() -> Terms {
        Terms {
            network: crate::Network::Preprod,
            script: Hash28::from([1u8; 28]),
            asset: crate::Asset::Lovelace,
            provider: Hash28::from([2u8; 28]),
            close_period: subbit_core::Duration::from_secs(900),
            confirmation_depth: 3,
            min_iou_amount: 5_000,
            catalog: crate::Catalog {
                default: crate::ResourceInfo {
                    cost: crate::Cost::ByRequest(0),
                    description: "Access to a metered resource".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    max_timeout_seconds: 60,
                    service_name: None,
                    tags: None,
                    icon_url: None,
                },
                resources: Vec::new(),
            },
            reference_script: None,
        }
    }

    pub(crate) fn extra() -> SubbitExtra {
        SubbitExtra::subbit_v1(&terms(), None)
    }

    pub(crate) fn accept() -> SubbitAccept {
        SubbitAccept::new(&terms(), "1000".to_string(), 60, None)
    }

    fn resource() -> spec::ResourceInfo {
        spec::ResourceInfo {
            url: "https://example.com/article".to_string(),
            description: Some("Access to an article".to_string()),
            mime_type: Some("text/html".to_string()),
            service_name: None,
            tags: None,
            icon_url: None,
        }
    }

    #[test]
    fn accept_matches_the_shape_of_a_real_x402_accept() {
        let json = serde_json::to_value(accept()).unwrap();
        assert_eq!(json["scheme"], "batch-settlement");
        assert_eq!(json["network"], "cardano:preprod");
        assert_eq!(json["amount"], "1000");
        assert_eq!(json["payTo"], Hash28::from([1u8; 28]).to_string());
        assert_eq!(json["maxTimeoutSeconds"], 60);
        assert_eq!(json["extra"]["kind"], "subbit-v1");
    }

    #[test]
    fn extra_serializes_with_a_kind_tag_and_the_non_redundant_script_terms() {
        let json = serde_json::to_value(extra()).unwrap();
        assert_eq!(json["kind"], "subbit-v1");
        assert_eq!(json["close_period"], 900_000);
        assert_eq!(json["min_iou_amount"], 5_000);
        assert!(json["channel_status"].is_null());
    }

    #[test]
    fn corrective_extra_carries_channel_status() {
        let extra = SubbitExtra::subbit_v1(
            &terms(),
            Some(ChannelStatus {
                iou: subbit_core::Iou::new(7_000, subbit_core::Signature::from([0u8; 64])),
                spendable: Some(3_000),
                uncommitted: Some(0),
            }),
        );
        let json = serde_json::to_value(&extra).unwrap();
        assert!(json["channel_status"].is_object());
    }

    #[test]
    fn unrecognized_kind_is_captured_as_unknown() {
        let json = serde_json::json!({ "kind": "sponsored", "foo": "bar" });
        let extra: SubbitExtra = serde_json::from_value(json).unwrap();
        assert!(matches!(extra, SubbitExtra::Unknown(_)));
    }

    #[test]
    fn wrong_kind_with_a_matching_shape_is_still_unknown() {
        // Same fields `Standard` expects, but `kind` lies about it:
        // structural match alone must not be enough.
        let mut json = serde_json::to_value(extra()).unwrap();
        json["kind"] = serde_json::Value::String("sponsored".to_string());
        let extra: SubbitExtra = serde_json::from_value(json).unwrap();
        assert!(matches!(extra, SubbitExtra::Unknown(_)));
    }

    #[test]
    fn a_foreign_scheme_entry_still_parses_via_unknown_extra() {
        let json = serde_json::json!({
            "scheme": "exact",
            "network": "eip155:84532",
            "amount": "10000",
            "payTo": "0x209693Bc6afc0C5328bA36FaF03C514EF312287C",
            "maxTimeoutSeconds": 60,
            "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
            "extra": { "name": "USDC", "version": "2" }
        });
        let accept: SubbitAccept = serde_json::from_value(json).unwrap();
        assert!(matches!(accept.extra, SubbitExtra::Unknown(_)));
    }

    #[test]
    fn wraps_in_a_payment_required() {
        let required = Require {
            x402_version: 2,
            error: Some("PAYMENT-SIGNATURE header is required".to_string()),
            resource: resource(),
            accepts: vec![accept()],
            extensions: None,
        };
        let json = serde_json::to_value(&required).unwrap();
        assert_eq!(json["x402Version"], 2);
        assert_eq!(json["resource"]["url"], "https://example.com/article");
        assert_eq!(json["accepts"][0]["scheme"], "batch-settlement");
    }

    #[test]
    fn real_world_402_response_from_the_reference_spec() {
        // Lifted verbatim from x402-foundation/x402's own v2 spec
        // (`specs/x402-specification-v2.md`, section 5.1.1): `resource`
        // lives once at the top level, each `accepts[]` entry carries
        // `amount` (not `maxAmountRequired`) with no per-entry
        // `resource`/`description`/`mimeType` of its own.
        let json = serde_json::json!({
            "x402Version": 2,
            "error": "PAYMENT-SIGNATURE header is required",
            "resource": {
                "url": "https://api.example.com/premium-data",
                "description": "Access to premium market data",
                "mimeType": "application/json",
                "serviceName": "Example Market Data",
                "tags": ["market-data", "finance"],
                "iconUrl": "https://api.example.com/icon.png"
            },
            "accepts": [
                {
                    "scheme": "exact",
                    "network": "eip155:84532",
                    "amount": "10000",
                    "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
                    "payTo": "0x209693Bc6afc0C5328bA36FaF03C514EF312287C",
                    "maxTimeoutSeconds": 60,
                    "extra": { "name": "USDC", "version": "2" }
                }
            ],
            "extensions": {}
        });
        let required: Require = serde_json::from_value(json).unwrap();
        assert_eq!(
            required.resource.url,
            "https://api.example.com/premium-data"
        );
        assert_eq!(required.accepts.len(), 1);
        assert!(matches!(required.accepts[0].extra, SubbitExtra::Unknown(_)));
    }
}
