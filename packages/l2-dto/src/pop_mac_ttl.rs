use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use subbit_core::{Duration, MacSig, Signature, Tag, VerifyingKey, tbs};

/// `key`, `tag`, and an expiry: what a `Pop` or `Mac` proves live
/// possession of. An `Iou`'s own amount is its freshness proof,
/// cumulative and strictly increasing; this has no amount, so it
/// needs an expiry instead.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct KeyTagTtl {
    #[n(0)]
    pub key: VerifyingKey,
    #[n(1)]
    pub tag: Tag,
    #[n(2)]
    pub ttl: Duration,
}

impl KeyTagTtl {
    /// What actually gets signed: not this struct's own encoding, a
    /// domain-tagged wrapping of it (`tbs::subbit`). `PopTtl`/`MacTtl`'s
    /// `sig` covers this, not `KeyTagTtl` directly, verify against it
    /// accordingly.
    pub fn tbs(&self) -> Vec<u8> {
        tbs::subbit(self)
    }
}

/// Proof of possession: an Ed25519 signature over a `KeyTagTtl`.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PopTtl {
    #[n(0)]
    pub body: KeyTagTtl,
    #[n(1)]
    pub sig: Signature,
}

/// A server-issued session shortcut: a cheap MAC over a `KeyTagTtl`,
/// handed to a `Pop`-authenticated caller so its *next* request can skip
/// the heavier Ed25519 proof.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MacTtl {
    #[n(0)]
    pub body: KeyTagTtl,
    #[n(1)]
    pub sig: MacSig,
}

/// Proof this key is live: either a `PopTtl` or a `MacTtl`.
///
/// `key` here is the same `iou_key` that signs an `Iou`'s cumulative
/// amount elsewhere in this protocol, a different message shape
/// (`tag` + `amount`, not `key` + `tag` + `ttl`). Reusing one key to
/// sign more than one payload shape is standard here, but it means
/// the shapes must stay unambiguous: a valid `Iou` signature must
/// never pass as a valid `Pop`, or the reverse. `PopTtl`/`MacTtl`
/// stay concrete, fixed structs rather than instantiations of a
/// shared generic, so that scrutiny happens once here rather than
/// wherever a future shape gets parameterised.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PopMacTtl {
    #[n(0)]
    Pop(#[n(0)] PopTtl),
    #[n(1)]
    Mac(#[n(0)] MacTtl),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_tag_ttl() -> KeyTagTtl {
        KeyTagTtl {
            key: VerifyingKey::from([1u8; 32]),
            tag: Tag::from(b"t".to_vec()),
            ttl: Duration::from_secs(0),
        }
    }

    #[test]
    fn pop_roundtrips() {
        let pop = PopMacTtl::Pop(PopTtl {
            body: key_tag_ttl(),
            sig: Signature::from([0u8; 64]),
        });
        let bytes = minicbor::to_vec(&pop).expect("encode failed");
        let decoded: PopMacTtl = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(pop, decoded);
    }

    #[test]
    fn mac_roundtrips() {
        let mac = PopMacTtl::Mac(MacTtl {
            body: key_tag_ttl(),
            sig: MacSig::from([0u8; 20]),
        });
        let bytes = minicbor::to_vec(&mac).expect("encode failed");
        let decoded: PopMacTtl = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(mac, decoded);
    }
}
