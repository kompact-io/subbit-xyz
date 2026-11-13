//! x402's own generic wire vocabulary (x402 v2). Scheme- and
//! provider-agnostic; unspecified type params default to
//! `serde_json::Value`.

use serde::{Deserialize, Serialize};

/// The `PAYMENT-REQUIRED` envelope, generic over the concrete
/// `accepts.extra` entry type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired<A = serde_json::Value> {
    pub x402_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub resource: ResourceInfo,
    pub accepts: Vec<PaymentRequirements<A>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// One `accepts[]` entry: one way to pay for a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements<A = serde_json::Value> {
    pub scheme: String,
    /// CAIP-2 format.
    pub network: String,
    /// Atomic token units.
    pub amount: String,
    pub pay_to: String,
    pub max_timeout_seconds: u64,
    /// Token contract address, or ISO 4217 currency code for fiat.
    pub asset: String,
    pub extra: A,
}

/// The `PAYMENT-SIGNATURE` envelope: `accepted` echoes the
/// `accepts[]` entry being paid under, `payload` is the scheme's own
/// wire data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<A = serde_json::Value, P = serde_json::Value> {
    pub x402_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    /// The `PaymentRequirements` entry being paid under.
    pub accepted: A,
    pub payload: P,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// The `PAYMENT-RESPONSE` result, on both success and failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// `extensions`'s own `#[serde(default)]` makes serde's derive infer an
// `E: Default` bound it doesn't actually need (`Option<E>` is `Default`
// regardless of `E`); spelling the real bound out overrides that.
#[serde(bound(serialize = "E: Serialize", deserialize = "E: Deserialize<'de>"))]
pub struct SettlementResponse<E = serde_json::Value> {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    /// Non-empty on success; empty, not `None`, when nothing was
    /// broadcast.
    pub transaction: String,
    /// CAIP-2 format.
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<E>,
}
