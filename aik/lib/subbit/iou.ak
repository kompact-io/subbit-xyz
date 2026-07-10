use aiken/cbor
use aiken/crypto.{VerificationKey, verify_ed25519_signature}
use subbit/prelude.{Amount, Signature}
use subbit/types as t

pub fn verify(
  key: VerificationKey,
  tag: t.Tag,
  amount: Amount,
  signature: Signature,
) {
  let message: Data = (tag, amount)
  verify_ed25519_signature(key, cbor.serialise(message), signature)
}
