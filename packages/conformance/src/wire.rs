use proptest::prelude::*;

use crate::AikenFn;
use subbit_core::{Constants, Currency, Datum, Redeemer, Stage};

fn currency_fn() -> AikenFn {
    AikenFn::from_shortcut("wire/currency")
}

#[test]
fn currency_ada_conforms() {
    assert!(currency_fn().eval_true(&Currency::Ada));
}

#[test]
fn currency_by_class_conforms() {
    assert!(currency_fn().eval_true(&Currency::Asset {
        hash: [1u8; 28],
        name: vec![1, 2, 3]
    }));
}

proptest! {
    #[test]
    fn prop_currency_conforms(currency: Currency) {
        assert!(currency_fn().eval_true(&currency));
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
