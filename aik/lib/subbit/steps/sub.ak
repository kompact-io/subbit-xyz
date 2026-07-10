use aiken/crypto.{Signature, VerificationKey}
use cardano/assets.{Value}
use subbit/assets.{amount_in, amount_out} as assets_
use subbit/iou
use subbit/prelude.{Amount}
use subbit/types as t

pub fn do(
  constants: t.Constants,
  value_in: Value,
  stage_in: t.Stage,
  value_out: Value,
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
