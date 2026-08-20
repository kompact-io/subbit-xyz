#![cfg(feature = "test-utils")]

use proptest::prelude::*;

#[allow(unused_imports)]
use crate::{Constants, Currency, Datum, Redeemer, Stage};

#[allow(dead_code)]
fn assert_roundtrip<T>(value: &T)
where
    T: minicbor::Encode<()> + for<'b> minicbor::Decode<'b, ()> + PartialEq + std::fmt::Debug,
{
    let bytes = minicbor::to_vec(value).expect("encode failed");
    let decoded: T = minicbor::decode(&bytes).expect("decode failed");
    assert_eq!(value, &decoded, "roundtrip mismatch, cbor = {bytes:x?}");
}

proptest! {
    #[test]
    fn roundtrip_currency(currency: Currency) {
        assert_roundtrip(&currency);
    }
    #[test]
    fn roundtrip_constants(constants: Constants) {
        assert_roundtrip(&constants);
    }
    #[test]
    fn roundtrip_stage(stage: Stage) {
        assert_roundtrip(&stage);
    }
    #[test]
    fn roundtrip_datum(datum: Datum) {
        assert_roundtrip(&datum);
    }
    #[test]
    fn roundtrip_redeemer(redeemer: Redeemer) {
        assert_roundtrip(&redeemer);
    }
}

proptest! {
    #[test]
    fn sanity_check_roundtrip_fails_on_corruption(currency: Currency) {
        let mut bytes = minicbor::to_vec(&currency).expect("encode failed");
        // flip a byte partway through to guarantee a mismatch (or decode error)
        let corrupt_at = bytes.len() / 2;
        if let Some(b) = bytes.get_mut(corrupt_at) {
            *b ^= 0xFF;
        }
        let decoded: Result<Currency, _> = minicbor::decode(&bytes);
        if let Ok(d) = decoded { assert_ne!(currency, d, "corruption didn't change the value — bad test data?") }
    }
}
