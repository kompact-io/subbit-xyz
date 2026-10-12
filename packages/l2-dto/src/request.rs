use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{ChannelId, Mutual, PopMacTtl};

/// Consumer -> Provider. Each dialect's own `Request` (vanilla's a
/// re-export, x402's a wrapper around this as its `payload` field)
/// takes its name from this.
///
/// Two ways to name which channel this is about, never both: `ById`
/// states it directly; `ByPopMac` doesn't, `auth`'s own key/tag
/// already is the channel id. Keeping each to one field rules out the
/// two ever disagreeing about which channel is meant.
///
/// JSON is untagged and every field flattened in: `ById`/`ByPopMac`
/// and `Commit`'s own variants are told apart by which fields are
/// actually present (`iou_key`/`tag` vs `auth`; `amount`/`signature`
/// vs `transaction`).
#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Request {
    /// Suitable when IOU or signed tx can act as auth.
    /// Warning :: Care should be taken when supporting this method.
    /// PopMac provides a cheap spam filter, not given here.
    #[n(0)]
    ById {
        #[n(0)]
        #[cfg_attr(feature = "serde", serde(flatten))]
        channel_id: ChannelId,
        #[n(1)]
        #[cfg_attr(feature = "serde", serde(flatten))]
        commit: Commit,
    },
    /// `auth` proves liveness (and, for `Pop`, earns a fresh session
    /// `Mac` back); `commit` is optional, for a liveness-only request
    /// claiming nothing new.
    #[n(1)]
    ByPopMac {
        #[n(0)]
        auth: PopMacTtl,
        #[n(1)]
        #[cfg_attr(feature = "serde", serde(flatten))]
        commit: Option<Commit>,
    },
}

/// Commit to movement of funds either by L2 IOU or L1 Tx.
#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Commit {
    #[n(0)]
    Iou(#[n(0)] subbit_core::Iou),
    #[n(1)]
    Mutual(#[n(0)] Mutual),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyTagTtl, PopTtl};
    use subbit_core::{Duration, Signature, Tag, VerifyingKey};

    fn channel_id() -> ChannelId {
        ChannelId {
            iou_key: VerifyingKey::from([0u8; 32]),
            tag: Tag::from(b"t".to_vec()),
        }
    }

    fn pop_auth() -> PopMacTtl {
        PopMacTtl::Pop(PopTtl {
            body: KeyTagTtl {
                key: VerifyingKey::from([0u8; 32]),
                tag: Tag::from(b"t".to_vec()),
                ttl: Duration::from_secs(0),
            },
            sig: Signature::from([0u8; 64]),
        })
    }

    #[test]
    fn by_id_iou_roundtrips_over_cbor() {
        let payload = Request::ById {
            channel_id: channel_id(),
            commit: Commit::Iou(subbit_core::Iou::new(1_000, Signature::from([0u8; 64]))),
        };
        let bytes = minicbor::to_vec(&payload).expect("encode failed");
        let decoded: Request = minicbor::decode(&bytes).expect("decode failed");
        assert!(matches!(
            decoded,
            Request::ById {
                commit: Commit::Iou(_),
                ..
            }
        ));
    }

    #[test]
    fn by_id_mutual_roundtrips_over_cbor() {
        let payload = Request::ById {
            channel_id: channel_id(),
            commit: Commit::Mutual(Mutual {
                transaction: vec![0xde, 0xad],
            }),
        };
        let bytes = minicbor::to_vec(&payload).expect("encode failed");
        let decoded: Request = minicbor::decode(&bytes).expect("decode failed");
        match decoded {
            Request::ById {
                commit: Commit::Mutual(Mutual { transaction }),
                ..
            } => assert_eq!(transaction, vec![0xde, 0xad]),
            _ => panic!("expected ById Mutual"),
        }
    }

    #[test]
    fn by_pop_mac_with_no_claim_roundtrips_over_cbor() {
        let payload = Request::ByPopMac {
            auth: pop_auth(),
            commit: None,
        };
        let bytes = minicbor::to_vec(&payload).expect("encode failed");
        let decoded: Request = minicbor::decode(&bytes).expect("decode failed");
        assert!(matches!(decoded, Request::ByPopMac { commit: None, .. }));
    }

    #[test]
    fn by_pop_mac_complementing_an_iou_roundtrips_over_cbor() {
        let payload = Request::ByPopMac {
            auth: pop_auth(),
            commit: Some(Commit::Iou(subbit_core::Iou::new(
                1_000,
                Signature::from([0u8; 64]),
            ))),
        };
        let bytes = minicbor::to_vec(&payload).expect("encode failed");
        let decoded: Request = minicbor::decode(&bytes).expect("decode failed");
        assert!(matches!(
            decoded,
            Request::ByPopMac {
                commit: Some(Commit::Iou(_)),
                ..
            }
        ));
    }

    #[cfg(feature = "x402")]
    mod json {
        use super::*;

        #[test]
        fn by_id_iou_flattens_with_no_tag() {
            let payload = Request::ById {
                channel_id: channel_id(),
                commit: Commit::Iou(subbit_core::Iou::new(1_000, Signature::from([0u8; 64]))),
            };
            let json = serde_json::to_value(&payload).unwrap();
            assert!(json.get("type").is_none(), "{json}");
            assert!(json.get("channel_id").is_none(), "{json}");
            assert!(json.get("commit").is_none(), "{json}");
            assert_eq!(json["amount"], 1_000);
            let decoded: Request = serde_json::from_value(json).unwrap();
            assert!(matches!(
                decoded,
                Request::ById {
                    commit: Commit::Iou(_),
                    ..
                }
            ));
        }

        #[test]
        fn by_id_mutual_flattens_transaction() {
            let payload = Request::ById {
                channel_id: channel_id(),
                commit: Commit::Mutual(Mutual {
                    transaction: vec![0xde, 0xad],
                }),
            };
            let json = serde_json::to_value(&payload).unwrap();
            assert_eq!(json["transaction"], "dead");
            let decoded: Request = serde_json::from_value(json).unwrap();
            match decoded {
                Request::ById {
                    commit: Commit::Mutual(Mutual { transaction }),
                    ..
                } => assert_eq!(transaction, vec![0xde, 0xad]),
                _ => panic!("expected ById Mutual"),
            }
        }

        #[test]
        fn by_pop_mac_with_no_claim_omits_inner_fields() {
            let payload = Request::ByPopMac {
                auth: pop_auth(),
                commit: None,
            };
            let json = serde_json::to_value(&payload).unwrap();
            assert!(json.get("amount").is_none(), "{json}");
            assert!(json.get("transaction").is_none(), "{json}");
            let decoded: Request = serde_json::from_value(json).unwrap();
            assert!(matches!(decoded, Request::ByPopMac { commit: None, .. }));
        }

        #[test]
        fn by_pop_mac_complementing_an_iou_flattens_both() {
            let payload = Request::ByPopMac {
                auth: pop_auth(),
                commit: Some(Commit::Iou(subbit_core::Iou::new(
                    1_000,
                    Signature::from([0u8; 64]),
                ))),
            };
            let json = serde_json::to_value(&payload).unwrap();
            assert!(json["auth"].is_object());
            assert_eq!(json["amount"], 1_000);
            let decoded: Request = serde_json::from_value(json).unwrap();
            assert!(matches!(
                decoded,
                Request::ByPopMac {
                    commit: Some(Commit::Iou(_)),
                    ..
                }
            ));
        }
    }
}
