use aiken/crypto.{VerificationKey}
use aiken/interval.{Finite, NegativeInfinity}
use cardano/assets.{Assets}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/fixtures as fx
use subbit/prelude.{Timebound}
use subbit/types as t

pub fn do(
  upper_bound: Timebound,
  constants: t.Constants,
  value_in: Assets,
  stage_in: t.Stage,
  value_out: Assets,
  stage_out: t.Stage,
) -> VerificationKey {
  let (_tag, currency, _iou_key, consumer, _provider, close_period) = constants
  expect t.Opened { subbed } = stage_in
  expect t.Closed { subbed: subbed_, elapse_at } = stage_out
  expect subbed == subbed_
  expect amount_in(currency, value_in) <= amount_out(currency, value_out)
  expect Finite(upper_bound) = upper_bound
  expect upper_bound <= elapse_at - close_period
  consumer
}

test close_succeeds_at_the_boundary() {
  do(
    Finite(0),
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(100),
    t.Closed { subbed: 0, elapse_at: fx.close_period },
  ) == fx.consumer_vkh
}

test close_fails_when_subbed_changes() fail {
  expect _vkey = do(
    Finite(0),
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(100),
    t.Closed { subbed: 1, elapse_at: fx.close_period },
  )
  True
}

test close_fails_when_value_decreases() fail {
  expect _vkey = do(
    Finite(0),
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(99),
    t.Closed { subbed: 0, elapse_at: fx.close_period },
  )
  True
}

test close_fails_when_upper_bound_not_finite() fail {
  expect _vkey = do(
    NegativeInfinity,
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(100),
    t.Closed { subbed: 0, elapse_at: fx.close_period },
  )
  True
}

test close_fails_when_too_late() fail {
  expect _vkey = do(
    Finite(1),
    fx.constants,
    assets.from_lovelace(100),
    t.Opened { subbed: 0 },
    assets.from_lovelace(100),
    t.Closed { subbed: 0, elapse_at: fx.close_period },
  )
  True
}
