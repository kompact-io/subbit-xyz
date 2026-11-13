use aiken/crypto.{VerificationKey}
use aiken/interval.{Finite, NegativeInfinity}
use subbit/fixtures as fx
use subbit/prelude.{Timebound}
use subbit/types as t

pub fn do(
  lower_bound: Timebound,
  constants: t.Constants,
  stage_in: t.Stage,
) -> VerificationKey {
  let (_tag, _currency, _iou_key, consumer, _provider, _close_period) = constants
  expect t.Closed { elapse_at, .. } = stage_in
  expect Finite(lower_bound) = lower_bound
  expect elapse_at <= lower_bound
  consumer
}

test elapse_succeeds_at_the_boundary() {
  do(Finite(100), fx.constants, t.Closed { subbed: 0, elapse_at: 100 }) == fx.consumer_vkh
}

test elapse_fails_before_elapse_at() fail {
  expect _vkey = do(
    Finite(99),
    fx.constants,
    t.Closed { subbed: 0, elapse_at: 100 },
  )
  True
}

test elapse_fails_when_lower_bound_not_finite() fail {
  expect _vkey = do(
    NegativeInfinity,
    fx.constants,
    t.Closed { subbed: 0, elapse_at: 100 },
  )
  True
}

test elapse_fails_when_not_closed() fail {
  expect _vkey = do(Finite(100), fx.constants, t.Settled)
  True
}
