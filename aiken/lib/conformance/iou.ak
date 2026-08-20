use aiken/crypto.{VerificationKey}
use subbit/iou
use subbit/prelude.{Amount, Signature}
use subbit/types as t

pub type Verify {
  key: VerificationKey,
  tag: t.Tag,
  amount: Amount,
  signature: Signature,
}

pub fn verify(v: Verify) {
  iou.verify(v.key, v.tag, v.amount, v.signature)
}
