use aiken/crypto.{VerificationKey}
use subbit/fixtures as fx
use subbit/types as t

pub fn do(constants: t.Constants, stage_in: t.Stage) -> VerificationKey {
  let (_tag, _currency, _iou_key, consumer, _provider, _close_period) = constants
  expect t.Settled = stage_in
  consumer
}

test end_succeeds_when_settled() {
  do(fx.constants, t.Settled) == fx.consumer_vkh
}

test end_fails_when_not_settled() fail {
  expect _vkey = do(fx.constants, t.Opened { subbed: 0 })
  True
}
