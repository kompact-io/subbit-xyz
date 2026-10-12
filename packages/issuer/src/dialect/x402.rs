//! x402's wire framing: `PAYMENT-SIGNATURE` out (base64 JSON
//! `Request`), `PAYMENT-REQUIRED`/`PAYMENT-RESPONSE` in (base64 JSON
//! `Require`/`Response`), matching `subbit-server`'s own
//! `dialect::x402`.
//!
//! A non-2xx reply can carry either of two shapes: a settlement
//! failure (`PAYMENT-RESPONSE`, `success: false`), or a fresh/
//! corrective challenge (`PAYMENT-REQUIRED`, the channel's real
//! status riding in `extra` on the corrective case). [`response`]
//! takes whichever header the caller actually found and handles it.
//!
//! Building a payment (`request`) needs to know which terms it's
//! paying under (`accepted`): there's no wire-level discovery inside
//! this module itself, a caller must have already seen one
//! `PAYMENT-REQUIRED` (via `response`, which hands the `accepts[]`
//! entry back for exactly this) before it can call `request` at all.

use base64::Engine;
use subbit_l2_dto::ChannelStatus;
use subbit_l2_dto::dialect::x402::{
    KnownExtra, Payload, Request, Require, Response, SCHEME, SubbitAccept, SubbitExtra, extension,
};

use crate::Issuer;
use crate::issuer::ResponseError;

pub static PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";
pub static PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";
pub static PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to decode response header")]
    Decode,
    #[error("server error: {0}")]
    Protocol(String),
    #[error(transparent)]
    Core(#[from] ResponseError),
}

/// Base64 (standard, padded) `PAYMENT-SIGNATURE` envelope committing
/// `issuer`'s next claim under `accepted`, the terms a prior
/// `response` call handed back (see this module's own doc comment).
pub fn request(issuer: &mut Issuer, cost: u64, accepted: SubbitAccept) -> String {
    let payload: Payload = issuer.next_payload(cost);
    let envelope = Request {
        x402_version: 2,
        resource: None,
        accepted,
        payload,
        extensions: None,
    };
    let bytes = serde_json::to_vec(&envelope).expect("Request always encodes to JSON");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Apply whichever header the caller found: `settlement` (the
/// `PAYMENT-RESPONSE` header, present on both a clean success and a
/// plain failure) takes priority when both are somehow present,
/// `required` (`PAYMENT-REQUIRED`) otherwise. `accepted_cache` is
/// updated as a side effect whenever `required` carries a
/// recognizable accept, regardless of whether the overall result is
/// `Ok` or `Err`: a corrective challenge is still an `Err` for the
/// caller (nothing to retry with yet without a bigger top-up), but
/// the terms it names are still exactly what the next real `request`
/// should pay under.
pub fn response(
    issuer: &mut Issuer,
    accepted_cache: &mut Option<SubbitAccept>,
    settlement: Option<&[u8]>,
    required: Option<&[u8]>,
) -> Result<(), Error> {
    if let Some(bytes) = settlement {
        return apply_settlement(issuer, decode(bytes)?);
    }
    if let Some(bytes) = required {
        return apply_required(issuer, accepted_cache, decode(bytes)?);
    }
    Err(Error::Decode)
}

fn decode<T: serde::de::DeserializeOwned>(base64_json: &[u8]) -> Result<T, Error> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_json)
        .map_err(|_| Error::Decode)?;
    serde_json::from_slice(&bytes).map_err(|_| Error::Decode)
}

fn apply_settlement(issuer: &mut Issuer, resp: Response) -> Result<(), Error> {
    let success = resp.success;
    let error_reason = resp.error_reason.clone();
    let ext = extension(&resp);
    if success {
        let status = ext.clone().and_then(|e| e.channel).ok_or(Error::Decode)?;
        let mac = ext.and_then(|e| e.mac);
        return Ok(issuer.apply_status(status, mac)?);
    }
    // A failure can still carry a real status (e.g. exceeded
    // capacity): commit it before surfacing the error, so the next
    // request can size a correct top-up off it.
    if let Some(status) = ext.and_then(|e| e.channel) {
        issuer.apply_status(status, None)?;
    }
    Err(Error::Protocol(error_reason.unwrap_or_default()))
}

fn apply_required(
    issuer: &mut Issuer,
    accepted_cache: &mut Option<SubbitAccept>,
    required: Require,
) -> Result<(), Error> {
    let accept = required.accepts.into_iter().find(|a| a.scheme == SCHEME);
    if let Some(accept) = &accept {
        *accepted_cache = Some(accept.clone());
    }
    if let Some(status) = accept.as_ref().and_then(channel_status) {
        issuer.apply_status(status, None)?;
    }
    Err(Error::Protocol(
        required
            .error
            .unwrap_or_else(|| "payment required".to_string()),
    ))
}

fn channel_status(accept: &SubbitAccept) -> Option<ChannelStatus> {
    match &accept.extra {
        SubbitExtra::Known(KnownExtra::SubbitV1 { channel_status, .. }) => channel_status.clone(),
        SubbitExtra::Unknown(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use subbit_core::{Duration, Hash28, Tag};
    use subbit_l2_dto::ChannelStatus;
    use subbit_l2_dto::dialect::x402::{SubbitAccept, settlement};

    use super::*;
    use crate::Account;

    fn issuer() -> Issuer {
        let account = Account::new(SigningKey::from_bytes(&[1u8; 32]), Tag::from(b"t".to_vec()));
        Issuer::new(account, Duration::from_secs(60))
    }

    fn status_for(issuer: &Issuer, amount: u64, spendable: Option<u64>) -> ChannelStatus {
        ChannelStatus {
            iou: issuer.account().iou(amount),
            spendable,
            uncommitted: None,
        }
    }

    fn terms() -> subbit_l2_dto::Terms {
        subbit_l2_dto::Terms {
            network: subbit_l2_dto::Network::Preprod,
            script: Hash28::from([1u8; 28]),
            asset: subbit_l2_dto::Asset::Lovelace,
            provider: Hash28::from([2u8; 28]),
            close_period: Duration::from_secs(900),
            confirmation_depth: 3,
            min_iou_amount: 5_000,
            catalog: subbit_l2_dto::Catalog {
                default: subbit_l2_dto::ResourceInfo {
                    cost: subbit_l2_dto::Cost::ByRequest(0),
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

    fn subbit_accept(channel_status: Option<ChannelStatus>) -> SubbitAccept {
        SubbitAccept::new(&terms(), "1000".to_string(), 60, channel_status)
    }

    fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
        let json = serde_json::to_vec(value).unwrap();
        base64::engine::general_purpose::STANDARD
            .encode(json)
            .into_bytes()
    }

    #[test]
    fn request_produces_a_base64_json_payment_signature_envelope() {
        let mut issuer = issuer();
        let accepted = subbit_accept(None);
        let envelope = request(&mut issuer, 50, accepted);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&envelope)
            .unwrap();
        let decoded: Request = serde_json::from_slice(&bytes).unwrap();
        assert!(matches!(
            decoded.payload,
            Payload::ById {
                commit: subbit_l2_dto::Commit::Iou(_),
                ..
            }
        ));
        assert_eq!(decoded.accepted.amount, "1000");
    }

    #[test]
    fn response_applies_a_successful_status_and_mac() {
        let mut issuer = issuer();
        let mut accepted = None;
        let body = encode(&settlement(
            subbit_l2_dto::Response {
                success: true,
                error: None,
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: Some(status_for(&issuer, 7_000, Some(3_000))),
                    mac: None,
                },
            },
            "keytag-1".to_string(),
            "cardano:preprod".to_string(),
        ));
        response(&mut issuer, &mut accepted, Some(&body), None).unwrap();
        assert_eq!(issuer.committed(), Some(7_000));
    }

    #[test]
    fn response_surfaces_a_failure_with_no_status_as_protocol_error() {
        let mut issuer = issuer();
        let mut accepted = None;
        let body = encode(&settlement(
            subbit_l2_dto::Response {
                success: false,
                error: Some(subbit_l2_dto::CoreError::InvalidIou),
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: None,
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        ));
        let err = response(&mut issuer, &mut accepted, Some(&body), None).unwrap_err();
        assert!(matches!(
            err,
            Error::Protocol(msg) if msg == subbit_l2_dto::CoreError::InvalidIou.to_string()
        ));
        assert_eq!(issuer.committed(), None);
    }

    #[test]
    fn response_still_commits_the_status_carried_by_a_capacity_failure() {
        let mut issuer = issuer();
        let mut accepted = None;
        let body = encode(&settlement(
            subbit_l2_dto::Response {
                success: false,
                error: Some(subbit_l2_dto::CoreError::Unspendable),
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: Some(status_for(&issuer, 7_000, Some(0))),
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        ));
        let err = response(&mut issuer, &mut accepted, Some(&body), None).unwrap_err();
        assert!(matches!(err, Error::Protocol(_)));
        assert_eq!(issuer.committed(), Some(7_000));
    }

    #[test]
    fn response_applies_the_corrective_status_carried_by_a_required_header() {
        let mut issuer = issuer();
        let mut accepted = None;
        let required = Require {
            x402_version: 2,
            error: Some("cumulative mismatch".to_string()),
            resource: subbit_l2_dto::dialect::x402::spec::ResourceInfo {
                url: "https://example.com/resource".to_string(),
                description: None,
                mime_type: None,
                service_name: None,
                tags: None,
                icon_url: None,
            },
            accepts: vec![subbit_accept(Some(status_for(&issuer, 7_000, Some(3_000))))],
            extensions: None,
        };
        let body = encode(&required);
        let err = response(&mut issuer, &mut accepted, None, Some(&body)).unwrap_err();
        assert!(matches!(err, Error::Protocol(msg) if msg == "cumulative mismatch"));
        assert_eq!(issuer.committed(), Some(7_000));
        assert!(accepted.is_some());
    }

    #[test]
    fn response_treats_an_initial_challenge_with_no_status_as_an_error_only() {
        let mut issuer = issuer();
        let mut accepted = None;
        let required = Require {
            x402_version: 2,
            error: Some("PAYMENT-SIGNATURE header is required".to_string()),
            resource: subbit_l2_dto::dialect::x402::spec::ResourceInfo {
                url: "https://example.com/resource".to_string(),
                description: None,
                mime_type: None,
                service_name: None,
                tags: None,
                icon_url: None,
            },
            accepts: vec![subbit_accept(None)],
            extensions: None,
        };
        let body = encode(&required);
        let err = response(&mut issuer, &mut accepted, None, Some(&body)).unwrap_err();
        assert!(matches!(err, Error::Protocol(_)));
        assert_eq!(issuer.committed(), None);
        // Still learned the terms, even though there was nothing to commit.
        assert!(accepted.is_some());
    }

    #[test]
    fn response_rejects_garbage_as_a_decode_error() {
        let mut issuer = issuer();
        let mut accepted = None;
        let err = response(
            &mut issuer,
            &mut accepted,
            Some(b"not base64 json at all"),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Decode));
    }
}
