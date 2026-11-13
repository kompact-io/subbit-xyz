use aiken/crypto.{VerificationKey}
use cardano/assets.{Assets}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/fixtures as fx
use subbit/types as t

pub fn do(
  constants: t.Constants,
  value_in: Assets,
  stage_in: t.Stage,
  value_out: Assets,
  stage_out: t.Stage,
) -> VerificationKey {
  let (_tag, currency, _iou_key, consumer, _provider, _close_period) = constants
  expect t.Opened { .. } = stage_in
  expect stage_in == stage_out
  expect amount_in(currency, value_in) < amount_out(currency, value_out)
  consumer
}

test add_succeeds_and_returns_consumer() {
  let stage = t.Opened { subbed: 0 }
  do(
    fx.constants,
    assets.from_lovelace(100),
    stage,
    assets.from_lovelace(150),
    stage,
  ) == fx.consumer_vkh
}

test add_fails_when_not_increasing() fail {
  let stage = t.Opened { subbed: 0 }
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(100),
    stage,
    assets.from_lovelace(100),
    stage,
  )
  True
}

test add_fails_when_stage_changes() fail {
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(150),
    t.Opened { subbed: 1 },
  )
  True
}

test add_fails_when_not_opened() fail {
  let stage = t.Closed { subbed: 0, elapse_at: 0 }
  expect _vkey = do(
    fx.constants,
    assets.from_lovelace(100),
    stage,
    assets.from_lovelace(150),
    stage,
  )
  True
}
