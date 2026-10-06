use aiken/crypto.{Signature, VerificationKey}
use cardano/assets.{Assets}
use subbit/assets.{amount_in, amount_out} as assets_
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
