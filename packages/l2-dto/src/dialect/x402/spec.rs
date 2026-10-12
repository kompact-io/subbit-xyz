//! x402's own generic wire vocabulary (x402 v2). Scheme- and
//! provider-agnostic; unspecified type params default to
//! `serde_json::Value`.

use serde::{Deserialize, Serialize};

/// The `PAYMENT-REQUIRED` envelope, generic over the concrete
/// `accepts.extra` entry type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired<A = serde_json::Value> {
    /// Protocol version identifier (must be 2).
    pub x402_version: u32,
    /// Why payment is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The protected resource.
    pub resource: ResourceInfo,
    /// Acceptable payment methods for this resource.
    pub accepts: Vec<PaymentRequirements<A>>,
    /// Protocol extensions data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// Describes the resource being protected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// URL of the protected resource.
    pub url: String,
    /// Human-readable description of the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// MIME type of the expected response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Human-readable name of the service hosting the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    /// Topical tags for the service, used for discovery filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// URL to an icon representing the service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// One `accepts[]` entry: one way to pay for a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements<A = serde_json::Value> {
    /// Payment scheme identifier (e.g. "exact").
    pub scheme: String,
    /// Blockchain network identifier in CAIP-2 format.
    pub network: String,
    /// Required payment amount, in atomic token units.
    pub amount: String,
    /// Recipient wallet address or role constant.
    pub pay_to: String,
    /// Maximum time allowed for payment completion.
    pub max_timeout_seconds: u64,
    /// Token contract address, or ISO 4217 currency code for fiat.
    pub asset: String,
    /// Scheme-specific extra data.
    pub extra: A,
}

/// The `PAYMENT-SIGNATURE` envelope: `accepted` echoes the
/// `accepts[]` entry being paid under, `payload` is the scheme's own
/// wire data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<A = serde_json::Value, P = serde_json::Value> {
    /// Protocol version identifier.
    pub x402_version: u32,
    /// The resource being accessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    /// The `PaymentRequirements` entry being paid under.
    pub accepted: A,
    /// Scheme-specific payment data.
    pub payload: P,
    /// Protocol extensions data.
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
    /// Whether settlement succeeded.
    pub success: bool,
    /// Why settlement failed (omitted if successful).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    /// Address of the payer's wallet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    /// Blockchain transaction hash. Non-empty on success; empty, not
    /// `None`, when nothing was broadcast.
    pub transaction: String,
    /// Blockchain network identifier in CAIP-2 format.
    pub network: String,
    /// The actual amount settled, in atomic units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    /// Protocol extensions data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<E>,
}
