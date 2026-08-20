use aiken/crypto.{VerificationKey}
use aiken/interval.{Finite}
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
