//! Vanilla's dialect: CBOR, a single `subbit` header carrying the
//! crate's shared `Request`/`Response` as base64, both ways. Thin:
//! vanilla re-exports both types as-is (see the crate root doc
//! comment), so there's nothing dialect-specific left here beyond the
//! header name and this wire envelope's own encoding; all of
//! `Ctx::handle_spend`/`Ctx::handle_refund`'s orchestration (catalog
//! lookup, the actual spend/refund) lives on `Ctx`, not here.

use http::{HeaderName, HeaderValue, StatusCode, header};
use subbit_l2_dto::{
    ChannelStatus, Error, MacTtl, Status,
    dialect::vanilla::{Request, Response},
};

use crate::{
    Keytag,
    dialect::{Dialect, Quote},
    wire::{self, Wire},
};

const REQUEST_HEADER_NAME: &str = "subbit";
const RESPONSE_HEADER_NAME: &str = "subbit";

pub struct Vanilla;

impl Dialect for Vanilla {
    fn recognizes(&self, headers: &http::HeaderMap) -> bool {
        wire::has_header(headers, REQUEST_HEADER_NAME)
    }

    fn decode(
        &self,
        headers: &http::HeaderMap,
        _quote: Quote,
    ) -> Result<subbit_l2_dto::Request, Wire> {
        headers
            .get(REQUEST_HEADER_NAME)
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| subbit_core::base64::from_base64::<Request>(raw).ok())
            .ok_or_else(|| wire::invalid_header_wire(REQUEST_HEADER_NAME))
    }

    /// Vanilla has no discovery/challenge step: as plain as `decode`'s
    /// own failure, nothing beyond the header it expects.
    fn challenge(&self, _quote: Quote) -> Wire {
        wire::missing_header_wire(REQUEST_HEADER_NAME)
    }

    fn spend_ok(
        &self,
        _quote: Quote,
        _keytag: &Keytag,
        status: ChannelStatus,
        mac: Option<MacTtl>,
    ) -> Wire {
        encode(Response {
            success: true,
            error: None,
            status: Status {
                channel: Some(status),
                mac,
            },
        })
    }

    fn refund_ok(&self, _quote: Quote, _keytag: &Keytag, status: ChannelStatus) -> Wire {
        encode(Response {
            success: true,
            error: None,
            status: Status {
                channel: Some(status),
                mac: None,
            },
        })
    }

    /// No corrective wire *shape* of its own (unlike x402's challenge),
    /// but `current` still rides in `status.channel` same as a success
    /// reply -- `Status::channel`'s own doc already covers this case
    /// ("a failure that still has a channel to report on"), so the
    /// client can resync instead of repeating a now-stale amount.
    fn error(&self, err: Error, _quote: Quote, current: Option<ChannelStatus>) -> Wire {
        encode(Response {
            success: false,
            error: Some(err),
            status: Status {
                channel: current,
                mac: None,
            },
        })
    }
}

/// On success the body is spoken for (by whatever real resource the
/// caller is metering), so the envelope rides in the `subbit` header,
/// as a request's did going in. On failure nothing else needs the
/// body: the envelope goes there instead, as plain CBOR.
fn encode(response: Response) -> Wire {
    let status_code = if response.success {
        StatusCode::OK
    } else {
        StatusCode::PAYMENT_REQUIRED
    };
    if response.success {
        match HeaderValue::from_str(&subbit_core::base64::to_base64(&response)) {
            Ok(value) => Wire::new(status_code, Vec::new())
                .with_header(HeaderName::from_static(RESPONSE_HEADER_NAME), value),
            Err(_) => Wire::new(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
        }
    } else {
        match minicbor::to_vec(&response) {
            Ok(bytes) => Wire::new(status_code, bytes).with_header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/cbor"),
            ),
            Err(_) => Wire::new(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use subbit_core::{Duration, Signature, Tag, VerifyingKey};
    use subbit_l2_dto::{
        Asset, ChannelId, Commit, KeyTagTtl, Network, PopMacTtl, PopTtl, Request, Terms,
    };

    use super::*;
    use crate::{
        Ctx, Keytag,
        catalog::{Catalog, CatalogDto},
        crypto::mac,
        db::Db,
        now,
    };

    fn catalog_dto(cost: subbit_l2_dto::Cost) -> CatalogDto {
        CatalogDto {
            resources: vec![(
                "/{*rest}".to_string(),
                subbit_l2_dto::ResourceInfo {
                    cost,
                    description: "Access to a metered resource".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    max_timeout_seconds: 60,
                    service_name: None,
                    tags: None,
                    icon_url: None,
                },
            )],
        }
    }

    fn test_terms() -> Terms {
        Terms {
            network: Network::Preprod,
            script: subbit_core::Hash28::from([0u8; 28]),
            asset: Asset::Lovelace,
            provider: subbit_core::Hash28::from([0u8; 28]),
            close_period: Duration::from_secs(900),
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
            vec![Arc::new(Vanilla)],
            None,
            10,
        )
    }

    fn pop_auth(key: VerifyingKey, sk: &[u8; 64], tag: Tag) -> PopMacTtl {
        let body = KeyTagTtl {
            key,
            tag,
            expires_at: now() + Duration::from_secs(60),
        };
        let sig = Signature::from(cryptoxide::ed25519::signature(&body.tbs(), sk));
        PopMacTtl::Pop(PopTtl { body, sig })
    }

    /// A liveness-only payload: `auth` present, nothing new claimed.
    fn auth_only(key: VerifyingKey, sk: &[u8; 64], tag: Tag) -> Request {
        Request::ByPopMac {
            auth: pop_auth(key, sk, tag),
            commit: None,
        }
    }

    fn headers_for(payload: &Request) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::HeaderName::from_static(REQUEST_HEADER_NAME),
            http::HeaderValue::from_str(&subbit_core::base64::to_base64(payload)).unwrap(),
        );
        headers
    }

    fn decode_wire(wire: &Wire) -> Response {
        let header_value = wire
            .headers
            .iter()
            .find(|(name, _)| name.as_str() == RESPONSE_HEADER_NAME)
            .map(|(_, value)| value.to_str().unwrap());
        match header_value {
            Some(raw) => subbit_core::base64::from_base64(raw).unwrap(),
            None => minicbor::decode(&wire.body).unwrap(),
        }
    }

    #[test]
    fn spend_deducts_cost_and_issues_a_mac_for_pop() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[1u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let payload = auth_only(key, &sk, tag);
        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        assert_eq!(wire.status, StatusCode::OK);
        let response = decode_wire(&wire);
        assert_eq!(response.status.channel.unwrap().spendable, Some(90));
        assert!(response.status.mac.is_some());
    }

    #[test]
    fn spend_accepts_an_iou_payload_too() {
        // An `Iou` payload needs no separate liveness proof, its own
        // signature is the authorization.
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[3u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let tbs = subbit_core::TagTbs::new(tag.clone(), 120).to_vec();
        let signature = Signature::from(cryptoxide::ed25519::signature(&tbs, &sk));
        let payload = Request::ById {
            channel_id: ChannelId {
                iou_key: key,
                tag: tag.clone(),
            },
            commit: Commit::Iou(subbit_core::Iou::new(120, signature)),
        };

        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        let response = decode_wire(&wire);
        assert_eq!(response.status.channel.unwrap().spendable, Some(110));
    }

    #[test]
    fn old_iou_rejection_still_carries_the_real_current_status() {
        // Regression test: a stale (non-strictly-increasing) `Iou` amount
        // used to get a bare `OldIou` error with `status.channel: None`
        // (see `Vanilla::error`'s own doc, and `Ctx::dialect_error`'s --
        // both used to special-case `InsufficientFunds` only). The client
        // has no way to resync without this, so it just repeats the same
        // stale amount forever -- see `plan-canary-l2-oldiou-backoff.md`.
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[5u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        // Same amount already committed -- `delta == 0`, `OldIou`.
        let tbs = subbit_core::TagTbs::new(tag.clone(), 100).to_vec();
        let signature = Signature::from(cryptoxide::ed25519::signature(&tbs, &sk));
        let payload = Request::ById {
            channel_id: ChannelId {
                iou_key: key,
                tag: tag.clone(),
            },
            commit: Commit::Iou(subbit_core::Iou::new(100, signature)),
        };

        let wire = ctx.handle_spend(&headers_for(&payload), "/resource");
        assert_eq!(wire.status, StatusCode::PAYMENT_REQUIRED);
        let response = decode_wire(&wire);
        assert_eq!(response.error, Some(subbit_l2_dto::Error::OldIou));
        let status = response
            .status
            .channel
            .expect("a backed channel's rejection should still carry its real current status");
        assert_eq!(status.iou.amount(), 100);
    }

    #[test]
    fn missing_header_is_payment_required() {
        let ctx = ctx();
        let wire = ctx.handle_spend(&http::HeaderMap::new(), "/resource");
        assert_eq!(wire.status, StatusCode::PAYMENT_REQUIRED);
    }

    #[test]
    fn refund_gives_back_spent_amount() {
        let ctx = ctx();
        let (sk, pk) = cryptoxide::ed25519::keypair(&[2u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);
        let payload = auth_only(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        let payload = auth_only(key, &sk, tag);
        let wire = ctx.handle_refund(&headers_for(&payload), "/resource");
        let response = decode_wire(&wire);
        assert_eq!(response.status.channel.unwrap().spendable, Some(100));
    }

    #[test]
    fn refund_gives_back_only_the_unused_portion_of_an_egress_quote() {
        let catalog = crate::catalog::Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByEgress(
            subbit_l2_dto::CostByEgress {
                base: 5,
                per_kib: 1,
                max_bytes: 10_240,
            },
        )))
        .unwrap();
        let ctx = ctx_with_catalog(catalog);
        let (sk, pk) = cryptoxide::ed25519::keypair(&[9u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        // Quote: 5 + 1*ceil(10240/1024) = 15, billed in full by spend.
        let payload = auth_only(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        // Real response was 1 KiB: actual cost 5 + 1*1 = 6, so 9 comes back.
        let payload = auth_only(key, &sk, tag);
        let mut headers = headers_for(&payload);
        headers.insert(http::header::CONTENT_LENGTH, "1024".parse().unwrap());
        let wire = ctx.handle_refund(&headers, "/resource");
        let response = decode_wire(&wire);
        assert_eq!(response.status.channel.unwrap().spendable, Some(100 - 6));
    }

    #[test]
    fn refund_with_no_input_gives_back_the_full_quote() {
        let catalog = crate::catalog::Catalog::new(catalog_dto(subbit_l2_dto::Cost::ByEgress(
            subbit_l2_dto::CostByEgress {
                base: 5,
                per_kib: 1,
                max_bytes: 10_240,
            },
        )))
        .unwrap();
        let ctx = ctx_with_catalog(catalog);
        let (sk, pk) = cryptoxide::ed25519::keypair(&[10u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"t".to_vec());
        ctx.seed_channel_for_test(&Keytag::new(key, tag.clone()), 100);

        let payload = auth_only(key, &sk, tag.clone());
        ctx.handle_spend(&headers_for(&payload), "/resource");

        // Nothing was served at all: full quote (15) comes back.
        let payload = auth_only(key, &sk, tag);
        let wire = ctx.handle_refund(&headers_for(&payload), "/resource");
        let response = decode_wire(&wire);
        assert_eq!(response.status.channel.unwrap().spendable, Some(100));
    }
}
