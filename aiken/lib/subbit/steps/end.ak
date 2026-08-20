use aiken/crypto.{VerificationKey}
use subbit/types as t

pub fn do(constants: t.Constants, stage_in: t.Stage) -> VerificationKey {
  let (_tag, _currency, _iou_key, consumer, _provider, _close_period) = constants
  expect t.Settled = stage_in
  consumer
}
