use aiken/crypto.{ScriptHash, Signature, VerificationKey, VerificationKeyHash}
use subbit/prelude.{Amount, Timestamp}

pub type Tag =
  ByteArray

pub type Currency {
  Ada
  Asset { hash: ScriptHash, name: ByteArray }
}

// Unwrap with 
// let (tag, currency, iou_key, consumer, provider, close_period) = constants

pub type Constants =
  (
    Tag,
    Currency,
    VerificationKey,
    VerificationKeyHash,
    VerificationKeyHash,
    Int,
  )

// Flatten 
// { own_hash: ScriptHash, constants: Constants, stage: Stage, }
// Unwrap with: 
// let (own_hash, constants, stage) = datum
pub type Datum =
  (ScriptHash, Constants, Stage)

pub type Stage {
  Opened { subbed: Amount }
  Closed { subbed: Amount, elapse_at: Timestamp }
  Settled
}

pub type Redeemer {
  Defer
  Main(Steps)
  Mutual
}

pub type Step {
  StepCont(Cont)
  StepEol(Eol)
}

pub type Steps =
  List<Step>

pub type Cont {
  Add
  Sub { n: Amount, sig: Signature }
  Close
  Settle { n: Amount, sig: Signature }
}

pub type Eol {
  End
  Elapse
}
