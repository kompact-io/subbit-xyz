//! x402's wire framing: `PAYMENT-SIGNATURE` out (base64 JSON
//! `Request`), `PAYMENT-REQUIRED`/`PAYMENT-RESPONSE` in (base64 JSON
//! `Require`/`Response`), matching `subbit-gate`'s own
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

use crate::Stamp;
use crate::stamp::ResponseError;

/// What to do about a `PAYMENT-REQUIRED` challenge naming terms to
/// pay under next -- same shape as `catalog::MissPolicy`: a plain
/// closure is a complete policy, and whether this is the very first
/// terms ever learned (`previous: None`) or a change from terms
/// already accepted (`previous: Some`) is for the policy itself to
/// look at, not split into separate hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermsDecision {
    /// Accept `accept` and pay under it from here on.
    Accept,
    /// Refuse it -- `accepted_cache` stays as it was (unset, or
    /// whatever was accepted before), so the caller's next request
    /// still goes out bare/under the old terms rather than silently
    /// adopting the new ones.
    Reject,
}

pub trait TermsPolicy: Send + Sync {
    fn on_terms(&self, accept: &SubbitAccept, previous: Option<&SubbitAccept>) -> TermsDecision;
}

impl<F> TermsPolicy for F
where
    F: Fn(&SubbitAccept, Option<&SubbitAccept>) -> TermsDecision + Send + Sync,
{
    fn on_terms(&self, accept: &SubbitAccept, previous: Option<&SubbitAccept>) -> TermsDecision {
        self(accept, previous)
    }
}

/// Accept whatever terms a provider names, every time -- the
/// previous, hardcoded behavior of `apply_required`.
pub fn accept_always(_accept: &SubbitAccept, _previous: Option<&SubbitAccept>) -> TermsDecision {
    TermsDecision::Accept
}

/// Accept the first terms ever learned, reject any change away from
/// them -- the safe default: a provider that raises price (or
/// otherwise alters terms) mid-session via a fresh challenge doesn't
/// get silently followed.
pub fn accept_once(_accept: &SubbitAccept, previous: Option<&SubbitAccept>) -> TermsDecision {
    match previous {
        None => TermsDecision::Accept,
        Some(_) => TermsDecision::Reject,
    }
}

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
/// `stamp`'s next claim under `accepted`, the terms a prior
/// `response` call handed back (see this module's own doc comment).
pub fn request(stamp: &mut Stamp, cost: u64, accepted: SubbitAccept) -> String {
    let payload: Payload = stamp.next_request(cost);
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
/// recognizable accept that `terms` approves (see `apply_required`),
/// regardless of whether the overall result is `Ok` or `Err`: a
/// corrective challenge is still an `Err` for the caller (nothing to
/// retry with yet without a bigger top-up), but once approved, the
/// terms it names are still exactly what the next real `request`
/// should pay under.
pub fn response(
    stamp: &mut Stamp,
    accepted_cache: &mut Option<SubbitAccept>,
    terms: &dyn TermsPolicy,
    settlement: Option<&[u8]>,
    required: Option<&[u8]>,
) -> Result<(), Error> {
    if let Some(bytes) = settlement {
        return apply_settlement(stamp, decode(bytes)?);
    }
    if let Some(bytes) = required {
        return apply_required(stamp, accepted_cache, terms, decode(bytes)?);
    }
    Err(Error::Decode)
}

fn decode<T: serde::de::DeserializeOwned>(base64_json: &[u8]) -> Result<T, Error> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_json)
        .map_err(|_| Error::Decode)?;
    serde_json::from_slice(&bytes).map_err(|_| Error::Decode)
}

fn apply_settlement(stamp: &mut Stamp, resp: Response) -> Result<(), Error> {
    let success = resp.success;
    let error_reason = resp.error_reason.clone();
    let ext = extension(&resp);
    if success {
        let status = ext.clone().and_then(|e| e.channel).ok_or(Error::Decode)?;
        let mac = ext.and_then(|e| e.mac);
        return Ok(stamp.apply_status(status, mac)?);
    }
    // A failure can still carry a real status (e.g. exceeded
    // capacity): commit it before surfacing the error, so the next
    // request can size a correct top-up off it.
    if let Some(status) = ext.and_then(|e| e.channel) {
        stamp.apply_status(status, None)?;
    }
    // x402's `error_reason` is this binding's own vocabulary
    // (`subbit_l2_dto::dialect::x402::error`), not `CoreError`'s raw
    // `Display` string -- `vanilla` matches `CoreError` directly
    // instead, since it has no such translation.
    if error_reason.as_deref()
        == Some(subbit_l2_dto::dialect::x402::X402Error::AuthInvalid.as_str())
    {
        stamp.clear_mac();
    }
    Err(Error::Protocol(error_reason.unwrap_or_default()))
}

fn apply_required(
    stamp: &mut Stamp,
    accepted_cache: &mut Option<SubbitAccept>,
    terms: &dyn TermsPolicy,
    required: Require,
) -> Result<(), Error> {
    let accept = required.accepts.into_iter().find(|a| a.scheme == SCHEME);
    if let Some(accept) = &accept
        && terms.on_terms(accept, accepted_cache.as_ref()) == TermsDecision::Accept
    {
        *accepted_cache = Some(accept.clone());
    }
    // The channel's own status is read off the challenge's own accept
    // regardless of whether `terms` approved paying under it -- it's
    // just the channel's current, factual balance, not itself a term
    // to consent to.
    if let Some(status) = accept.as_ref().and_then(channel_status) {
        stamp.apply_status(status, None)?;
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
    use subbit_core::{Duration, Hash28, Instant, Tag};
    use subbit_l2_dto::ChannelStatus;
    use subbit_l2_dto::dialect::x402::{SubbitAccept, settlement};

    use super::*;
    use crate::Id;

    fn stamp() -> Stamp {
        let account = Id::new(SigningKey::from_bytes(&[1u8; 32]), Tag::from(b"t".to_vec()));
        Stamp::new(account, Duration::from_secs(60))
    }

    fn status_for(stamp: &Stamp, amount: u64, spendable: Option<u64>) -> ChannelStatus {
        ChannelStatus {
            iou: stamp.id().iou(amount),
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
                resources: vec![(
                    "/{*rest}".to_string(),
                    subbit_l2_dto::ResourceInfo {
                        cost: subbit_l2_dto::Cost::ByRequest(0),
                        description: "Access to a metered resource".to_string(),
                        mime_type: "application/octet-stream".to_string(),
                        max_timeout_seconds: 60,
                        service_name: None,
                        tags: None,
                        icon_url: None,
                    },
                )],
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
        let mut stamp = stamp();
        let accepted = subbit_accept(None);
        let envelope = request(&mut stamp, 50, accepted);
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
        let mut stamp = stamp();
        let mut accepted = None;
        let body = encode(&settlement(
            subbit_l2_dto::Response {
                success: true,
                error: None,
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: Some(status_for(&stamp, 7_000, Some(3_000))),
                    mac: None,
                },
            },
            "keytag-1".to_string(),
            "cardano:preprod".to_string(),
        ));
        response(&mut stamp, &mut accepted, &accept_always, Some(&body), None).unwrap();
        assert_eq!(stamp.committed(), Some(7_000));
    }

    /// Parity with `dialect::vanilla`'s own
    /// `response_clears_the_cached_mac_on_invalid_auth` -- x402 only
    /// has `error_reason` (a stringified `CoreError`) to recognize this
    /// from, not a typed match, but the reaction must be the same.
    #[test]
    fn response_clears_the_cached_mac_on_invalid_auth() {
        let mut stamp = stamp();
        let mut accepted = None;
        let mac = subbit_l2_dto::MacTtl {
            body: subbit_l2_dto::KeyTagTtl {
                key: subbit_core::VerifyingKey::from([9u8; 32]),
                tag: Tag::from(b"t".to_vec()),
                expires_at: Instant::from_secs(60),
            },
            sig: subbit_core::MacSig::from([0u8; 20]),
        };
        let ok_body = encode(&settlement(
            subbit_l2_dto::Response {
                success: true,
                error: None,
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: Some(status_for(&stamp, 100, Some(100))),
                    mac: Some(mac),
                },
            },
            "keytag-1".to_string(),
            "cardano:preprod".to_string(),
        ));
        response(
            &mut stamp,
            &mut accepted,
            &accept_always,
            Some(&ok_body),
            None,
        )
        .unwrap();
        assert!(matches!(stamp.auth(), subbit_l2_dto::PopMacTtl::Mac(_)));

        let failure_body = encode(&settlement(
            subbit_l2_dto::Response {
                success: false,
                error: Some(subbit_l2_dto::CoreError::InvalidAuth),
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: None,
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        ));
        let _ = response(
            &mut stamp,
            &mut accepted,
            &accept_always,
            Some(&failure_body),
            None,
        );
        assert!(matches!(stamp.auth(), subbit_l2_dto::PopMacTtl::Pop(_)));
    }

    #[test]
    fn response_surfaces_a_failure_with_no_status_as_protocol_error() {
        let mut stamp = stamp();
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
        let err =
            response(&mut stamp, &mut accepted, &accept_always, Some(&body), None).unwrap_err();
        assert!(matches!(
            err,
            Error::Protocol(msg) if msg == "invalid_payload"
        ));
        assert_eq!(stamp.committed(), None);
    }

    #[test]
    fn response_still_commits_the_status_carried_by_a_capacity_failure() {
        let mut stamp = stamp();
        let mut accepted = None;
        let body = encode(&settlement(
            subbit_l2_dto::Response {
                success: false,
                error: Some(subbit_l2_dto::CoreError::Unspendable),
                status: subbit_l2_dto::dialect::x402::SubbitExtension {
                    channel: Some(status_for(&stamp, 7_000, Some(0))),
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        ));
        let err =
            response(&mut stamp, &mut accepted, &accept_always, Some(&body), None).unwrap_err();
        assert!(matches!(err, Error::Protocol(_)));
        assert_eq!(stamp.committed(), Some(7_000));
    }

    #[test]
    fn response_applies_the_corrective_status_carried_by_a_required_header() {
        let mut stamp = stamp();
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
            accepts: vec![subbit_accept(Some(status_for(&stamp, 7_000, Some(3_000))))],
            extensions: None,
        };
        let body = encode(&required);
        let err =
            response(&mut stamp, &mut accepted, &accept_always, None, Some(&body)).unwrap_err();
        assert!(matches!(err, Error::Protocol(msg) if msg == "cumulative mismatch"));
        assert_eq!(stamp.committed(), Some(7_000));
        assert!(accepted.is_some());
    }

    #[test]
    fn response_treats_an_initial_challenge_with_no_status_as_an_error_only() {
        let mut stamp = stamp();
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
        let err =
            response(&mut stamp, &mut accepted, &accept_always, None, Some(&body)).unwrap_err();
        assert!(matches!(err, Error::Protocol(_)));
        assert_eq!(stamp.committed(), None);
        // Still learned the terms, even though there was nothing to commit.
        assert!(accepted.is_some());
    }

    #[test]
    fn response_rejects_garbage_as_a_decode_error() {
        let mut stamp = stamp();
        let mut accepted = None;
        let err = response(
            &mut stamp,
            &mut accepted,
            &accept_always,
            Some(b"not base64 json at all"),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Decode));
    }

    fn required_naming(accept: SubbitAccept) -> Require {
        Require {
            x402_version: 2,
            error: Some("payment required".to_string()),
            resource: subbit_l2_dto::dialect::x402::spec::ResourceInfo {
                url: "https://example.com/resource".to_string(),
                description: None,
                mime_type: None,
                service_name: None,
                tags: None,
                icon_url: None,
            },
            accepts: vec![accept],
            extensions: None,
        }
    }

    #[test]
    fn accept_once_learns_the_first_terms_it_ever_sees() {
        let mut stamp = stamp();
        let mut accepted = None;
        let body = encode(&required_naming(subbit_accept(None)));
        response(&mut stamp, &mut accepted, &accept_once, None, Some(&body)).unwrap_err();
        assert!(accepted.is_some());
    }

    #[test]
    fn accept_once_rejects_a_later_challenge_naming_different_terms() {
        let mut stamp = stamp();
        let mut accepted = None;
        let first = encode(&required_naming(subbit_accept(None)));
        response(&mut stamp, &mut accepted, &accept_once, None, Some(&first)).unwrap_err();
        let first_accepted = accepted.clone();

        let mut raised = subbit_accept(None);
        raised.amount = "999999".to_string();
        let second = encode(&required_naming(raised));
        response(&mut stamp, &mut accepted, &accept_once, None, Some(&second)).unwrap_err();

        // Still exactly what was first accepted -- the raised amount
        // never got in.
        assert_eq!(accepted.map(|a| a.amount), first_accepted.map(|a| a.amount));
    }
}
