//! x402's dialect: `PAYMENT-SIGNATURE` in (base64 JSON `Request`),
//! `PAYMENT-REQUIRED`/`PAYMENT-RESPONSE` out (base64 JSON
//! `Require`/`Response`), all headers, no body either
//! direction. All of `Ctx::handle_spend`/`Ctx::handle_refund`'s
//! orchestration lives on `Ctx`, not here.

use base64::Engine;
use http::StatusCode;
use subbit_l2_dto::{
    ChannelStatus, CoreError, MacTtl, Terms,
    dialect::x402::{
        Payload, Request, Require, SCHEME, SubbitAccept, SubbitExtension, settlement,
        spec::ResourceInfo,
    },
};

use crate::{
    Keytag,
    dialect::{Dialect, Quote},
    wire::{self, Wire},
};

/// x402 v2's three HTTP headers (`specs/transports-v2/http.md`'s own
/// "Header Summary"), standard (padded) base64 JSON each way.
const PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";
const PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";
const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

pub struct X402;

impl Dialect for X402 {
    fn recognizes(&self, headers: &http::HeaderMap) -> bool {
        wire::has_header(headers, PAYMENT_SIGNATURE_HEADER)
    }

    /// Decodes the envelope and checks `accepted` against `quote`
    /// (same scheme, network, and script this provider is currently
    /// offering for the url): a stale or mismatched echo is rejected
    /// here, same as a malformed envelope. `accepted.amount` itself is
    /// never checked against `quote.cost`: what this spend/refund
    /// actually costs is resolved server-side off `Catalog`, by `Ctx`,
    /// after `decode` returns.
    fn decode(&self, headers: &http::HeaderMap, quote: Quote) -> Result<Payload, Wire> {
        let envelope = headers
            .get(PAYMENT_SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| base64::engine::general_purpose::STANDARD.decode(raw).ok())
            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok());

        match envelope {
            Some(envelope) if accepted_matches(&envelope.accepted, &quote) => Ok(envelope.payload),
            _ => Err(wire::invalid_header_wire(PAYMENT_SIGNATURE_HEADER)),
        }
    }

    fn challenge(&self, quote: Quote) -> Wire {
        required(quote, None)
    }

    fn spend_ok(
        &self,
        quote: Quote,
        keytag: &Keytag,
        status: ChannelStatus,
        mac: Option<MacTtl>,
    ) -> Wire {
        settlement_wire(
            StatusCode::OK,
            keytag.to_string(),
            quote.terms,
            subbit_l2_dto::Response {
                success: true,
                error: None,
                status: SubbitExtension {
                    channel: Some(status),
                    mac,
                },
            },
        )
    }

    fn refund_ok(&self, quote: Quote, keytag: &Keytag, status: ChannelStatus) -> Wire {
        settlement_wire(
            StatusCode::OK,
            keytag.to_string(),
            quote.terms,
            subbit_l2_dto::Response {
                success: true,
                error: None,
                status: SubbitExtension {
                    channel: Some(status),
                    mac: None,
                },
            },
        )
    }

    /// On `Unspendable` with a channel to report on, this is a
    /// corrective `PAYMENT-REQUIRED` (same shape as `challenge`,
    /// `current` folded in) so the client can size its next attempt;
    /// anything else is a plain failed `Response`, no
    /// commitment identifier (nothing was committed).
    fn error(&self, err: CoreError, quote: Quote, current: Option<ChannelStatus>) -> Wire {
        match (err, current) {
            (CoreError::Unspendable, Some(status)) => required(quote, Some(status)),
            (err, current) => settlement_wire(
                StatusCode::PAYMENT_REQUIRED,
                String::new(),
                quote.terms,
                subbit_l2_dto::Response {
                    success: false,
                    error: Some(err),
                    status: SubbitExtension {
                        channel: current,
                        mac: None,
                    },
                },
            ),
        }
    }
}

/// `accepted` must name this provider's own scheme, network, and
/// script, the same ones `required` just advertised (or would, for
/// this url): nothing here checks `accepted.amount` against
/// `quote.cost`, that's not a decode-time concern, see `decode`'s own
/// doc comment.
fn accepted_matches(accepted: &SubbitAccept, quote: &Quote) -> bool {
    accepted.scheme == SCHEME
        && accepted.network == quote.terms.network.to_caip2()
        && accepted.pay_to == quote.terms.script.to_string()
}

/// The 402 signal itself: a fresh challenge (`channel_status: None`,
/// nothing to report on yet), or resent corrective, when a `voucher`/
/// `deposit`'s cumulative amount didn't match the provider's record.
fn required(quote: Quote, channel_status: Option<ChannelStatus>) -> Wire {
    let accept = SubbitAccept::new(
        quote.terms,
        quote.cost.to_string(),
        quote.resource.max_timeout_seconds,
        channel_status,
    );
    let resource = ResourceInfo {
        url: quote.resource_url,
        description: Some(quote.resource.description),
        mime_type: Some(quote.resource.mime_type),
        service_name: quote.resource.service_name,
        tags: quote.resource.tags,
        icon_url: quote.resource.icon_url,
    };
    let required = Require {
        x402_version: 2,
        error: None,
        resource,
        accepts: vec![accept],
        extensions: None,
    };
    header_wire(
        StatusCode::PAYMENT_REQUIRED,
        PAYMENT_REQUIRED_HEADER,
        &required,
    )
}

/// Builds and encodes a `Response` (see
/// `subbit_l2_dto::dialect::x402::settlement`'s own doc comment for
/// why `status` rides in `extensions` while `success`/`error` sit on
/// the envelope directly).
fn settlement_wire(
    status_code: StatusCode,
    transaction: String,
    terms: &Terms,
    response: subbit_l2_dto::Response,
) -> Wire {
    let settlement_response = settlement(response, transaction, terms.network.to_caip2());
    header_wire(status_code, PAYMENT_RESPONSE_HEADER, &settlement_response)
}

/// Base64 JSON into `name`, empty body: the wire shape every response
/// here uses, success or failure alike (see this module's own doc
/// comment for why).
fn header_wire(status: StatusCode, name: &'static str, body: &impl serde::Serialize) -> Wire {
    let encoded = match serde_json::to_vec(body) {
        Ok(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
        Err(_) => return Wire::new(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
    };
    match (
        http::HeaderName::from_bytes(name.as_bytes()),
        http::HeaderValue::from_str(&encoded),
    ) {
        (Ok(header_name), Ok(value)) => {
            Wire::new(status, Vec::new()).with_header(header_name, value)
        }
        _ => Wire::new(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use subbit_core::{Signature, Tag, VerifyingKey};
    use subbit_l2_dto::{
        Asset, ChannelId, Commit, KeyTagTtl, Network, PopMacTtl, PopTtl, Terms,
        dialect::x402::Response,
    };

    use super::*;
    use crate::{
        Ctx, Keytag,
        catalog::{Catalog, CatalogDto},
        crypto::mac,
        db::Db,
    };

    fn catalog_dto(cost: subbit_l2_dto::Cost) -> CatalogDto {
        CatalogDto {
            default: subbit_l2_dto::ResourceInfo {
                cost,
                description: "Access to a metered resource".to_string(),
                mime_type: "application/octet-stream".to_string(),
                max_timeout_seconds: 60,
                service_name: None,
                tags: None,
                icon_url: None,
            },
            resources: Vec::new(),
        }
    }

    fn test_terms() -> Terms {
        Terms {
            network: Network::Preprod,
            script: subbit_core::Hash28::from([0u8; 28]),
            asset: Asset::Lovelace,
            provider: subbit_core::Hash28::from([0u8; 28]),
            close_period: subbit_core::Duration::from_secs(900),
            confirmation_depth: 3,
            min_iou_amount: 0,
            catalog: catalog_dto(subbit_l2_dto::Cost::ByRequest(0)),
            reference_script: None,
        }
    }

    fn ctx() -> Ctx {
        let catalog = Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByRequest(10))).unwrap();
        ctx_with_catalog(catalog)
    }

    fn ctx_with_catalog(catalog: Catalog) -> Ctx {
        ctx_with_catalog_and_base_url(catalog, None)
    }

    fn ctx_with_catalog_and_base_url(catalog: Catalog, public_base_url: Option<String>) -> Ctx {
        let path = tempfile::NamedTempFile::new().unwrap();
        let db = Arc::new(
            Db::open(crate::db::Config {
                db_path: path.path().to_string_lossy().to_string(),
                ..Default::default()
            })
            .unwrap(),
        );
        let mac = Arc::new(mac::Mac::new(mac::Key::default()));
        Ctx::new(
            db,
            mac,
            Arc::new(catalog),
            Arc::new(test_terms()),
            Arc::new(crate::mutual::Unsupported),
            vec![Arc::new(X402)],
            public_base_url,
        )
    }

    fn pop_auth(key: VerifyingKey, sk: &[u8; 64], tag: Tag) -> Payload {
        let body = KeyTagTtl {
            key,
            tag,
            ttl: crate::now() + subbit_core::Duration::from_secs(60),
        };
        let sig = Signature::from(cryptoxide::ed25519::signature(&body.tbs(), sk));
        Payload::ByPopMac {
            auth: PopMacTtl::Pop(PopTtl { body, sig }),
            commit: None,
        }
    }

    /// Matches `test_terms()`'s own scheme/network/script: any test
    /// using `test_terms()` (every `ctx*` helper above does) can pay
    /// under this accept and pass `accepted_matches`. `amount` isn't
    /// checked at decode time (see `decode`'s own doc comment), so an
    /// arbitrary placeholder is fine here.
    fn accepted() -> SubbitAccept {
        SubbitAccept::new(&test_terms(), "0".to_string(), 60, None)
    }

    fn headers_for(payload: &Payload) -> http::HeaderMap {
        headers_for_accepted(payload, accepted())
    }

    fn headers_for_accepted(payload: &Payload, accepted: SubbitAccept) -> http::HeaderMap {
        let envelope = Request {
            x402_version: 2,
            resource: None,
            accepted,
            payload: payload.clone(),
            extensions: None,
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::HeaderName::from_bytes(PAYMENT_SIGNATURE_HEADER.as_bytes()).unwrap(),
            http::HeaderValue::from_str(&encoded).unwrap(),
        );
        headers
    }

    fn header_body(wire: &Wire, name: &str) -> Vec<u8> {
        let raw = wire
            .headers
            .iter()
            .find(|(header_name, _)| header_name.as_str().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.to_str().unwrap())
            .unwrap_or_else(|| panic!("missing {name} header"));
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .unwrap()
    }

    fn decode_settlement(wire: &Wire) -> Response {
        serde_json::from_slice(&header_body(wire, PAYMENT_RESPONSE_HEADER)).unwrap()
    }

    fn decode_required(wire: &Wire) -> Require {
        serde_json::from_slice(&header_body(wire, PAYMENT_REQUIRED_HEADER)).unwrap()
    }

    #[test]
    fn iou_payload_spends_against_the_matching_keytag() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[3u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        // The payload's own cumulative amount must exceed what's already
        // committed (100), an Iou never decreases, see `OldIou`.
        let tbs = subbit_core::TagTbs::new(tag.clone(), 120).to_vec();
        let signature = Signature::from(cryptoxide::ed25519::signature(&tbs, &sk));
        let payload = Payload::ById {
            channel_id: ChannelId {
                iou_key: key,
                tag: tag.clone(),
            },
            commit: Commit::Iou(subbit_core::Iou::new(120, signature)),
        };

        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        assert_eq!(wire.status, StatusCode::OK);
        let body = decode_settlement(&wire);
        assert!(body.success);
        assert!(!body.transaction.is_empty());
        let extension = subbit_l2_dto::dialect::x402::extension(&body).unwrap();
        assert_eq!(extension.channel.unwrap().spendable, Some(110));
        assert!(extension.mac.is_none());
    }

    #[test]
    fn pop_mac_payload_issues_a_fresh_mac() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[4u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let payload = pop_auth(key, &sk, tag);
        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        let body = decode_settlement(&wire);
        let extension = subbit_l2_dto::dialect::x402::extension(&body).unwrap();
        assert_eq!(extension.channel.unwrap().spendable, Some(90));
        assert!(extension.mac.is_some());
    }

    #[test]
    fn decode_rejects_a_payload_naming_the_wrong_network() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[7u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let mut mismatched = accepted();
        mismatched.network = "cardano:mainnet".to_string();
        let payload = pop_auth(key, &sk, tag);
        let wire = ctx.handle_spend(&headers_for_accepted(&payload, mismatched), "/resource");
        assert_eq!(wire.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn challenge_advertises_the_cost_for_this_url() {
        let ctx = ctx();
        let wire = ctx.handle_spend(&http::HeaderMap::new(), "/resource");
        assert_eq!(wire.status, StatusCode::PAYMENT_REQUIRED);
        let required = decode_required(&wire);
        assert_eq!(required.accepts[0].amount, "10");
    }

    #[test]
    fn challenge_resource_is_the_bare_path_without_a_configured_base_url() {
        let ctx = ctx();
        let wire = ctx.handle_spend(&http::HeaderMap::new(), "/resource");
        let required = decode_required(&wire);
        assert_eq!(required.resource.url, "/resource");
    }

    #[test]
    fn challenge_resource_is_fully_qualified_with_a_configured_base_url() {
        let catalog = Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByRequest(10))).unwrap();
        let ctx = ctx_with_catalog_and_base_url(catalog, Some("https://example.com".to_string()));
        let wire = ctx.handle_spend(&http::HeaderMap::new(), "/resource");
        let required = decode_required(&wire);
        assert_eq!(required.resource.url, "https://example.com/resource");
    }

    #[test]
    fn unspendable_rebuilds_a_corrective_challenge() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[6u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        // Not enough to cover the flat cost of 10.
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 5);

        let payload = pop_auth(key, &sk, tag);
        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        assert_eq!(wire.status, StatusCode::PAYMENT_REQUIRED);
        let required = decode_required(&wire);
        assert_eq!(required.accepts[0].amount, "10");
    }

    #[test]
    fn refund_gives_back_spent_amount() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[5u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let payload = pop_auth(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        let payload = pop_auth(key, &sk, tag);
        let wire = ctx.handle_refund(&headers_for(&payload), "/resource");
        let body = decode_settlement(&wire);
        let extension = subbit_l2_dto::dialect::x402::extension(&body).unwrap();
        assert_eq!(extension.channel.unwrap().spendable, Some(100));
    }

    #[test]
    fn refund_gives_back_only_the_unused_portion_of_an_egress_quote() {
        let catalog = Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByEgress(
            subbit_l2_dto::CostByEgress {
                base: 5,
                per_kib: 1,
                max_bytes: 10_240,
            },
        )))
        .unwrap();
        let ctx = ctx_with_catalog(catalog);
        let (sk, pk) = cryptoxide::ed25519::keypair(&[11u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        // Quote: 5 + 1*ceil(10240/1024) = 15, billed in full by spend.
        let payload = pop_auth(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        // Real response was 1 KiB: actual cost 5 + 1*1 = 6, so 9 comes back.
        let payload = pop_auth(key, &sk, tag);
        let mut headers = headers_for(&payload);
        headers.insert(http::header::CONTENT_LENGTH, "1024".parse().unwrap());
        let wire = ctx.handle_refund(&headers, "/resource");
        let body = decode_settlement(&wire);
        let extension = subbit_l2_dto::dialect::x402::extension(&body).unwrap();
        assert_eq!(extension.channel.unwrap().spendable, Some(100 - 6));
    }

    #[test]
    fn refund_gives_back_the_quote_minus_a_claimed_cost() {
        let catalog = Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByClaim(20))).unwrap();
        let ctx = ctx_with_catalog(catalog);
        let (sk, pk) = cryptoxide::ed25519::keypair(&[12u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let payload = pop_auth(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        // The application claimed 7 out of a 20 quote: 13 comes back.
        let payload = pop_auth(key, &sk, tag);
        let mut headers = headers_for(&payload);
        headers.insert(subbit_l2_dto::CLAIM_COST_HEADER, "7".parse().unwrap());
        let wire = ctx.handle_refund(&headers, "/resource");
        let body = decode_settlement(&wire);
        let extension = subbit_l2_dto::dialect::x402::extension(&body).unwrap();
        assert_eq!(extension.channel.unwrap().spendable, Some(100 - 7));
    }
}
