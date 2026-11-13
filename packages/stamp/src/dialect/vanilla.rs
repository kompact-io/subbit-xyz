//! Vanilla's wire framing: `subbit-core`'s own base64 CBOR
//! convention, carried in the `subbit` header. The provider-side
//! equivalent of this module is `subbit-gate`'s `dialect::vanilla`.

use subbit_core::base64::{from_base64, to_base64};
use subbit_l2_dto::{Error as CoreError, Status, dialect::vanilla::Response};

use crate::Stamp;
use crate::stamp::ResponseError;

pub static HEADER: &str = "subbit";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to decode response envelope")]
    Decode,
    #[error("server error: {0:?}")]
    Protocol(CoreError),
    #[error(transparent)]
    Core(#[from] ResponseError),
}

/// Base64 request envelope committing `stamp`'s next claim. Send it
/// yourself (native reqwest, JS fetch, ...); hand the reply to
/// [`response`]/[`response_bytes`].
pub fn request(stamp: &mut Stamp, cost: u64) -> String {
    to_base64(&stamp.next_request(cost))
}

/// Parse, verify, and apply a base64 response envelope from the
/// server. On success the envelope rides in the `subbit` header, use
/// this.
pub fn response(stamp: &mut Stamp, envelope: &str) -> Result<(), Error> {
    let result = from_base64::<Response>(envelope).map_err(|_| Error::Decode)?;
    apply(stamp, result)
}

/// As `response`, but for the envelope's raw CBOR bytes rather than its
/// base64 text. On failure, the envelope rides in the response body,
/// use this: a body has no need for base64's text-safety, so the
/// server sends it as plain bytes.
pub fn response_bytes(stamp: &mut Stamp, bytes: &[u8]) -> Result<(), Error> {
    let result: Response = minicbor::decode(bytes).map_err(|_| Error::Decode)?;
    apply(stamp, result)
}

fn apply(stamp: &mut Stamp, result: Response) -> Result<(), Error> {
    if result.success {
        let Status { channel, mac } = result.status;
        let status = channel.ok_or(Error::Decode)?;
        return Ok(stamp.apply_status(status, mac)?);
    }
    let err = result.error.unwrap_or(CoreError::Other);
    if err == CoreError::InvalidAuth {
        stamp.clear_mac();
    }
    // A rejection can still carry the channel's real current status
    // (gate attaches one whenever there's a real, backed channel behind
    // the error -- see `Ctx::dialect_error`'s own doc). Adopt it the
    // same way a success reply would (`apply_status` verifies the
    // iou's signature before trusting it) so the next attempt is sized
    // off gate's truth instead of repeating this same stale amount
    // forever. Concurrent requests from other seats racing this same
    // channel aren't this crate's concern -- `apply_status` already
    // only ever moves `committed` forward, never back.
    if let Status {
        channel: Some(status),
        mac,
    } = result.status
    {
        stamp.apply_status(status, mac)?;
    }
    Err(Error::Protocol(err))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use subbit_core::{Duration, Tag};
    use subbit_l2_dto::ChannelStatus;

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

    #[test]
    fn request_produces_a_payload_the_server_can_decode() {
        let mut stamp = stamp();
        let envelope = request(&mut stamp, 50);
        let decoded: subbit_l2_dto::Request = subbit_core::base64::from_base64(&envelope).unwrap();
        assert!(matches!(
            decoded,
            subbit_l2_dto::Request::ById {
                commit: subbit_l2_dto::Commit::Iou(_),
                ..
            }
        ));
    }

    fn success(status: ChannelStatus) -> Response {
        Response {
            success: true,
            error: None,
            status: Status {
                channel: Some(status),
                mac: None,
            },
        }
    }

    #[test]
    fn response_applies_a_successful_status() {
        let mut stamp = stamp();
        let result = success(status_for(&stamp, 7_000, Some(3_000)));
        let envelope = subbit_core::base64::to_base64(&result);
        response(&mut stamp, &envelope).unwrap();
        assert_eq!(stamp.committed(), Some(7_000));
    }

    #[test]
    fn response_bytes_applies_a_successful_status() {
        let mut stamp = stamp();
        let result = success(status_for(&stamp, 7_000, Some(3_000)));
        let bytes = minicbor::to_vec(&result).unwrap();
        response_bytes(&mut stamp, &bytes).unwrap();
        assert_eq!(stamp.committed(), Some(7_000));
    }

    #[test]
    fn response_resyncs_committed_from_an_old_iou_rejection() {
        // Parity with x402's own
        // `response_still_commits_the_status_carried_by_a_capacity_failure`:
        // a rejection that still carries the channel's real current status
        // (now true of `OldIou` too, see `Vanilla::error`'s gate-side own
        // doc) must be adopted, not discarded, so the next mint is sized
        // off gate's truth instead of repeating the same stale amount.
        let mut stamp = stamp();
        let result = Response {
            success: false,
            error: Some(CoreError::OldIou),
            status: Status {
                channel: Some(status_for(&stamp, 7_000, Some(3_000))),
                mac: None,
            },
        };
        let envelope = subbit_core::base64::to_base64(&result);
        let err = response(&mut stamp, &envelope).unwrap_err();
        assert!(matches!(err, Error::Protocol(CoreError::OldIou)));
        assert_eq!(stamp.committed(), Some(7_000));
    }

    #[test]
    fn response_rejects_garbage_as_a_decode_error() {
        let mut stamp = stamp();
        let err = response(&mut stamp, "not valid base64url cbor!!").unwrap_err();
        assert!(matches!(err, Error::Decode));
    }
}
