//! Vanilla's wire framing: `subbit-core`'s own base64 CBOR
//! convention, carried in the `subbit` header. The provider-side
//! equivalent of this module is `subbit-server`'s `dialect::vanilla`.

use subbit_core::base64::{from_base64, to_base64};
use subbit_l2_dto::{CoreError, Status, dialect::vanilla::Response};

use crate::Issuer;
use crate::issuer::ResponseError;

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

/// Base64 request envelope committing `issuer`'s next claim. Send it
/// yourself (native reqwest, JS fetch, ...); hand the reply to
/// [`response`]/[`response_bytes`].
pub fn request(issuer: &mut Issuer, cost: u64) -> String {
    to_base64(&issuer.next_payload(cost))
}

/// Parse, verify, and apply a base64 response envelope from the
/// server. On success the envelope rides in the `subbit` header, use
/// this.
pub fn response(issuer: &mut Issuer, envelope: &str) -> Result<(), Error> {
    let result = from_base64::<Response>(envelope).map_err(|_| Error::Decode)?;
    apply(issuer, result)
}

/// As `response`, but for the envelope's raw CBOR bytes rather than its
/// base64 text. On failure, the envelope rides in the response body,
/// use this: a body has no need for base64's text-safety, so the
/// server sends it as plain bytes.
pub fn response_bytes(issuer: &mut Issuer, bytes: &[u8]) -> Result<(), Error> {
    let result: Response = minicbor::decode(bytes).map_err(|_| Error::Decode)?;
    apply(issuer, result)
}

fn apply(issuer: &mut Issuer, result: Response) -> Result<(), Error> {
    if result.success {
        let Status { channel, mac } = result.status;
        let status = channel.ok_or(Error::Decode)?;
        return Ok(issuer.apply_status(status, mac)?);
    }
    let err = result.error.unwrap_or(CoreError::Other);
    if err == CoreError::InvalidAuth {
        issuer.clear_mac();
    }
    Err(Error::Protocol(err))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use subbit_core::{Duration, Tag};
    use subbit_l2_dto::ChannelStatus;

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

    #[test]
    fn request_produces_a_payload_the_server_can_decode() {
        let mut issuer = issuer();
        let envelope = request(&mut issuer, 50);
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
        let mut issuer = issuer();
        let result = success(status_for(&issuer, 7_000, Some(3_000)));
        let envelope = subbit_core::base64::to_base64(&result);
        response(&mut issuer, &envelope).unwrap();
        assert_eq!(issuer.committed(), Some(7_000));
    }

    #[test]
    fn response_bytes_applies_a_successful_status() {
        let mut issuer = issuer();
        let result = success(status_for(&issuer, 7_000, Some(3_000)));
        let bytes = minicbor::to_vec(&result).unwrap();
        response_bytes(&mut issuer, &bytes).unwrap();
        assert_eq!(issuer.committed(), Some(7_000));
    }

    #[test]
    fn response_clears_the_cached_mac_on_invalid_auth() {
        let mut issuer = issuer();
        let status = status_for(&issuer, 100, Some(100));
        issuer
            .apply_status(
                status,
                Some(subbit_l2_dto::MacTtl {
                    body: subbit_l2_dto::KeyTagTtl {
                        key: subbit_core::VerifyingKey::from([9u8; 32]),
                        tag: Tag::from(b"t".to_vec()),
                        ttl: Duration::from_secs(60),
                    },
                    sig: subbit_core::MacSig::from([0u8; 20]),
                }),
            )
            .unwrap();
        assert!(matches!(issuer.auth(), crate::Auth::Mac(_)));

        let failure = Response {
            success: false,
            error: Some(CoreError::InvalidAuth),
            status: Status {
                channel: None,
                mac: None,
            },
        };
        let envelope = subbit_core::base64::to_base64(&failure);
        let err = response(&mut issuer, &envelope).unwrap_err();
        assert!(matches!(err, Error::Protocol(CoreError::InvalidAuth)));
        assert!(matches!(issuer.auth(), crate::Auth::Pop(_)));
    }

    #[test]
    fn response_rejects_garbage_as_a_decode_error() {
        let mut issuer = issuer();
        let err = response(&mut issuer, "not valid base64url cbor!!").unwrap_err();
        assert!(matches!(err, Error::Decode));
    }
}
