//! Decodes a CIP-30 `signTx`'s own return shape -- a standalone
//! `transaction_witness_set` CBOR blob, not a complete transaction
//! (CIP-30 always returns just the witness set, regardless of its own
//! `partialSign` flag) -- down to the one vkey+signature pair
//! `Wallet::sign_tx`'s own contract promises. Pure CBOR, no JS: the
//! actual CIP-30 call lives wherever that's wired up (`subbit-console`'s
//! own wasm layer), not here -- this is the one piece of that decode
//! worth being plain, native-testable Rust rather than buried, untested,
//! inside a wasm-only file.

use cardano_sdk::{Signature, VerificationKey, cbor};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cbor decode error: {0}")]
    Cbor(#[from] cbor::decode::Error),
    #[error("transaction_witness_set carries no vkey witness (key 0)")]
    NoVkeyWitness,
    #[error("vkey witness's own key/signature isn't the expected 32/64 bytes")]
    BadLength,
}

/// The first `vkeywitness` (`[vkey, signature]`) under a
/// `transaction_witness_set`'s own key `0` -- the only one we ever
/// expect, since every `Wallet::sign_tx` caller here signs with
/// exactly one address's key. Every other witness-set field (native
/// scripts, Plutus data/redeemers/scripts, bootstrap witnesses) is
/// skipped over, regardless of its own shape or position in the map --
/// none of those are ever part of what a single-address signing
/// request returns.
pub fn decode_single_vkey_witness(bytes: &[u8]) -> Result<(VerificationKey, Signature), Error> {
    let mut d = cbor::Decoder::new(bytes);
    let len = d.map()?;
    let mut seen: u64 = 0;
    loop {
        if let Some(n) = len {
            if seen >= n {
                break;
            }
        } else if d.datatype()? == cbor::data::Type::Break {
            d.skip()?;
            break;
        }

        let key: u8 = d.decode()?;
        if key == 0 {
            d.array()?; // `[+ vkeywitness]`
            d.array()?; // the first `vkeywitness = [vkey, signature]`
            let vkey: [u8; 32] = d.bytes()?.try_into().map_err(|_| Error::BadLength)?;
            let sig: [u8; 64] = d.bytes()?.try_into().map_err(|_| Error::BadLength)?;
            return Ok((VerificationKey::from(vkey), Signature::from(sig)));
        }
        d.skip()?;
        seen += 1;
    }
    Err(Error::NoVkeyWitness)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_sdk::cbor::Encoder;

    fn encode_single_witness_set(vkey: [u8; 32], sig: [u8; 64]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut e = Encoder::new(&mut buf);
        e.map(1).unwrap();
        e.u8(0).unwrap();
        e.array(1).unwrap();
        e.array(2).unwrap();
        e.bytes(&vkey).unwrap();
        e.bytes(&sig).unwrap();
        buf
    }

    #[test]
    fn decodes_the_sole_vkey_witness() {
        let vkey = [7u8; 32];
        let sig = [9u8; 64];
        let bytes = encode_single_witness_set(vkey, sig);
        let (decoded_vkey, decoded_sig) = decode_single_vkey_witness(&bytes).unwrap();
        assert_eq!(<[u8; 32]>::from(decoded_vkey), vkey);
        assert_eq!(<[u8; 64]>::from(decoded_sig), sig);
    }

    #[test]
    fn missing_vkey_witness_errors() {
        let mut buf = Vec::new();
        let mut e = Encoder::new(&mut buf);
        e.map(0).unwrap();
        let err = decode_single_vkey_witness(&buf);
        assert!(matches!(err, Err(Error::NoVkeyWitness)));
    }

    #[test]
    fn skips_unrelated_keys_before_finding_the_vkey_witness() {
        // key 5 (plutus_data) deliberately comes first -- not
        // canonical-CBOR key order, proving this doesn't assume key 0
        // is first.
        let vkey = [1u8; 32];
        let sig = [2u8; 64];
        let mut buf = Vec::new();
        let mut e = Encoder::new(&mut buf);
        e.map(2).unwrap();
        e.u8(5).unwrap();
        e.bytes(&[0u8; 4]).unwrap();
        e.u8(0).unwrap();
        e.array(1).unwrap();
        e.array(2).unwrap();
        e.bytes(&vkey).unwrap();
        e.bytes(&sig).unwrap();

        let (decoded_vkey, decoded_sig) = decode_single_vkey_witness(&buf).unwrap();
        assert_eq!(<[u8; 32]>::from(decoded_vkey), vkey);
        assert_eq!(<[u8; 64]>::from(decoded_sig), sig);
    }
}
