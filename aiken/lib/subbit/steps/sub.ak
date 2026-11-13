use aiken/crypto.{Signature, VerificationKey}
use cardano/assets.{Assets}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/fixtures as fx
use subbit/iou
use subbit/prelude.{Amount}
use subbit/types as t

pub fn do(
  constants: t.Constants,
  value_in: Assets,
  stage_in: t.Stage,
  value_out: Assets,
  stage_out: t.Stage,
  owed: Amount,
  signature: Signature,
) -> VerificationKey {
  trace @"sub.do": @"starting sub step"
  let (tag, currency, iou_key, _consumer, provider, _close_period) = constants
  expect t.Opened { subbed: subbed_in } = stage_in
  expect t.Opened { subbed: subbed_out } = stage_out
  expect
    subbed_out - subbed_in == amount_in(currency, value_in) - amount_out(
      currency,
      value_out,
    )
  expect iou.verify(iou_key, tag, owed, signature)
  expect subbed_out <= owed
  provider
}

test sub_succeeds_and_returns_provider() {
  do(
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(90),
    t.Opened { subbed: 10 },
    fx.owed_1000,
    fx.sig_for_1000,
  ) == fx.provider_vkh
}

test sub_fails_when_delta_mismatches_subbed() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(95),
    t.Opened { subbed: 10 },
    fx.owed_1000,
    fx.sig_for_1000,
  )
  True
}

test sub_fails_with_bad_signature() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(90),
    t.Opened { subbed: 10 },
    fx.owed_1000,
    fx.bad_sig,
  )
  True
}

test sub_fails_when_subbed_exceeds_owed() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(1100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(0),
    t.Opened { subbed: 1100 },
    fx.owed_1000,
    fx.sig_for_1000,
  )
  True
}
