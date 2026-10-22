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
  let (tag, currency, iou_key, _consumer, provider, _close_period) = constants
  trace @"settle.do": tag
  expect t.Closed { subbed: subbed_in, .. } = stage_in
  expect t.Settled = stage_out
  expect iou.verify(iou_key, tag, owed, signature)
  let amount_taken = amount_in(currency, value_in) - amount_out(
    currency,
    value_out,
  )
  let owed_relative = owed - subbed_in
  expect amount_taken <= owed_relative
  provider
}

test settle_succeeds_and_returns_provider() {
  do(
    fx.constants,
    assets.from_lovelace(500),
    t.Closed { subbed: 0, elapse_at: 0 },
    assets.from_lovelace(1),
    t.Settled,
    fx.owed_1000,
    fx.sig_for_1000,
  ) == fx.provider_vkh
}

test settle_fails_with_bad_signature() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(500),
    t.Closed { subbed: 0, elapse_at: 0 },
    assets.from_lovelace(0),
    t.Settled,
    fx.owed_1000,
    fx.bad_sig,
  )
  True
}

test settle_fails_when_taking_more_than_owed() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(1001),
    t.Closed { subbed: 0, elapse_at: 0 },
    assets.from_lovelace(0),
    t.Settled,
    fx.owed_1000,
    fx.sig_for_1000,
  )
  True
}

test settle_respects_already_subbed_amount() {
  do(
    fx.constants,
    assets.from_lovelace(500),
    t.Closed { subbed: 400, elapse_at: 0 },
    assets.from_lovelace(1),
    t.Settled,
    fx.owed_1000,
    fx.sig_for_1000,
  ) == fx.provider_vkh
}

test settle_fails_when_not_closed() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(500),
    t.Opened { subbed: 0 },
    assets.from_lovelace(0),
    t.Settled,
    fx.owed_1000,
    fx.sig_for_1000,
  )
  True
}

test settle_fails_when_not_settled() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(500),
    t.Closed { subbed: 0, elapse_at: 0 },
    assets.from_lovelace(0),
    t.Closed { subbed: 0, elapse_at: 0 },
    fx.owed_1000,
    fx.sig_for_1000,
  )
  True
}
