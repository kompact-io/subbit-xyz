use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{ChannelStatus, Error, MacTtl};

/// The part of `Response` x402 has no field of its own for, so it
/// rides in `extensions` there; vanilla carries it inline.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Status {
    /// Present on success, and on a failure that still has a channel
    /// to report on (e.g. exceeded capacity).
    #[n(0)]
    pub channel: Option<ChannelStatus>,
    /// Present only after a `Pop`-authenticated request.
    #[n(1)]
    pub mac: Option<MacTtl>,
}

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Response {
    #[n(0)]
    pub success: bool,
    #[n(1)]
    pub error: Option<Error>,
    #[n(2)]
    pub status: Status,
}

#[cfg(test)]
mod tests {
    use super::*;
    use subbit_core::{Iou, Signature};

    #[test]
    fn success_roundtrips_over_cbor() {
        let response = Response {
            success: true,
            error: None,
            status: Status {
                channel: Some(ChannelStatus {
                    iou: Iou::new(7_000, Signature::from([0u8; 64])),
                    spendable: Some(3_000),
                    uncommitted: Some(0),
                }),
                mac: None,
            },
        };
        let bytes = minicbor::to_vec(&response).expect("encode failed");
        let decoded: Response = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(response, decoded);
    }

    #[test]
    fn failure_roundtrips_over_cbor() {
        let response = Response {
            success: false,
            error: Some(Error::InvalidAuth),
            status: Status {
                channel: None,
                mac: None,
            },
        };
        let bytes = minicbor::to_vec(&response).expect("encode failed");
        let decoded: Response = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(response, decoded);
    }
}
