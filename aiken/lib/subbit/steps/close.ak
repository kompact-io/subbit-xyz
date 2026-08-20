use aiken/crypto.{VerificationKey}
use aiken/interval.{Finite}
use cardano/assets.{Value}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/prelude.{Timebound}
use subbit/types as t

pub fn do(
  upper_bound: Timebound,
  constants: t.Constants,
  value_in: Value,
  stage_in: t.Stage,
  value_out: Value,
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
