use aiken/crypto.{VerificationKey}
use cardano/assets.{Value}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/types as t

pub fn do(
  constants: t.Constants,
  value_in: Value,
  stage_in: t.Stage,
  value_out: Value,
  stage_out: t.Stage,
) -> VerificationKey {
  let (_tag, currency, _iou_key, consumer, _provider, _close_period) = constants
  expect t.Opened { .. } = stage_in
  expect stage_in == stage_out
  expect amount_in(currency, value_in) < amount_out(currency, value_out)
  consumer
}
