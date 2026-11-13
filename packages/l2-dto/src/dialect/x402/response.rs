pub type Response = super::spec::SettlementResponse<SubbitExtension>;

pub type SubbitExtension = crate::Status;

/// `status` rides in `extensions`, x402 has no field for it.
/// `transaction` is `batch-settlement`'s commitment identifier; no
/// `payer` field, Subbit identifies the payer by `Keytag` already.
pub fn settlement(response: crate::Response, transaction: String, network: String) -> Response {
    let error_reason = response.error.as_ref().map(super::error_reason);
    Response {
        success: response.success,
        error_reason,
        payer: None,
        transaction,
        network,
        amount: None,
        extensions: Some(response.status),
    }
}

/// The inverse of `settlement`.
pub fn extension(response: &Response) -> Option<SubbitExtension> {
    response.extensions.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelStatus, Error, MacTtl};
    use subbit_core::{Iou, Signature};

    fn status() -> ChannelStatus {
        ChannelStatus {
            iou: Iou::new(7_000, Signature::from([0u8; 64])),
            spendable: Some(3_000),
            uncommitted: Some(0),
        }
    }

    #[test]
    fn success_carries_status_in_extensions() {
        let response = settlement(
            crate::Response {
                success: true,
                error: None,
                status: SubbitExtension {
                    channel: Some(status()),
                    mac: None,
                },
            },
            "keytag-123".to_string(),
            "cardano:preprod".to_string(),
        );
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["transaction"], "keytag-123");
        assert!(json["extensions"]["channel"].is_object());
        assert_eq!(extension(&response).unwrap().channel, Some(status()));
    }

    #[test]
    fn pop_authenticated_pop_mac_gets_a_mac_back() {
        let mac = MacTtl {
            body: crate::KeyTagTtl {
                key: subbit_core::VerifyingKey::from([1u8; 32]),
                tag: subbit_core::Tag::from(b"t".to_vec()),
                expires_at: subbit_core::Instant::from_secs(60),
            },
            sig: subbit_core::MacSig::from([0u8; 20]),
        };
        let response = settlement(
            crate::Response {
                success: true,
                error: None,
                status: SubbitExtension {
                    channel: None,
                    mac: Some(mac),
                },
            },
            "keytag-123".to_string(),
            "cardano:preprod".to_string(),
        );
        let json = serde_json::to_value(&response).unwrap();
        assert!(json["extensions"]["mac"].is_object());
        assert!(extension(&response).unwrap().mac.is_some());
    }

    #[test]
    fn failure_carries_no_transaction_when_nothing_was_committed() {
        let response = settlement(
            crate::Response {
                success: false,
                error: Some(Error::InvalidAuth),
                status: SubbitExtension {
                    channel: None,
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        );
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(
            json["errorReason"],
            "invalid_batch_settlement_cardano_auth_invalid"
        );
        assert_eq!(json["transaction"], "");
    }

    #[test]
    fn exceeds_capacity_failure_still_carries_status() {
        let response = settlement(
            crate::Response {
                success: false,
                error: Some(Error::InsufficientFunds),
                status: SubbitExtension {
                    channel: Some(status()),
                    mac: None,
                },
            },
            String::new(),
            "cardano:preprod".to_string(),
        );
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["success"], false);
        assert!(json["extensions"]["channel"].is_object());
    }

    #[test]
    fn extension_is_none_when_absent() {
        let response = Response {
            success: true,
            error_reason: None,
            payer: None,
            transaction: "keytag-123".to_string(),
            network: "cardano:preprod".to_string(),
            amount: None,
            extensions: None,
        };
        assert!(extension(&response).is_none());
    }
}
