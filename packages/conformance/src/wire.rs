use proptest::prelude::*;

use crate::AikenFn;
use subbit_l1_dto::{Asset, Constants, Datum, Redeemer, Stage};

fn asset_fn() -> AikenFn {
    AikenFn::from_shortcut("wire/asset")
}

#[test]
fn asset_lovelace_conforms() {
    assert!(asset_fn().eval_true(&Asset::Lovelace));
}

#[test]
fn asset_native_conforms() {
    assert!(asset_fn().eval_true(&Asset::Native {
        hash: [1u8; 28],
        name: vec![1, 2, 3]
    }));
}

proptest! {
    #[test]
    fn prop_asset_conforms(asset: Asset) {
        assert!(asset_fn().eval_true(&asset));
    }

    #[test]
    fn prop_constants_conforms(constants: Constants) {
        assert!(AikenFn::from_shortcut("wire/constants").eval_true(&constants));
    }

    #[test]
    fn prop_stage_conforms(stage: Stage) {
        assert!(AikenFn::from_shortcut("wire/stage").eval_true(&stage));
    }

    #[test]
    fn prop_datum_conforms(datum: Datum) {
        assert!(AikenFn::from_shortcut("wire/datum").eval_true(&datum));
    }

    #[test]
    fn prop_redeemer_conforms(redeemer: Redeemer) {
        assert!(AikenFn::from_shortcut("wire/redeemer").eval_true(&redeemer));
    }
}
