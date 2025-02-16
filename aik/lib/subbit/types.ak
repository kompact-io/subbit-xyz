use aiken/crypto.{ScriptHash, Signature, VerificationKey, VerificationKeyHash}

pub type Amount =
  Int

pub type Timestamp =
  Int

pub type SubbitId =
  ByteArray

pub type Currency {
  Ada
  ByHash(ScriptHash)
  ByClass(ScriptHash, ByteArray)
}

pub type Constants {
  subbit_id: SubbitId,
  currency: Currency,
  iou_key: VerificationKey,
  consumer: VerificationKeyHash,
  provider: VerificationKeyHash,
  close_period: Int,
}

pub type Datum {
  own_hash: ScriptHash,
  stage: Stage,
}

pub type Stage {
  Opened(Constants, Amount)
  Closed(Constants, Amount, Timestamp)
  Settled(VerificationKeyHash)
}

// Alternative consideration 
// pub type StageAlt {
//   StageMid(Constants, MidStage) 
//   StageEnd(VerificationKeyHash)
// }
// 
// pub type MidStage {
//   Open_(OpenParams)
//   Closed_(ClosedParams)
// }
//
pub type Redeemer {
  Batch
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
  Sub(Iou)
  Close
  Settle(Iou)
}

pub type Eol {
  End
  Expire
}

pub type Iou {
  amount: Amount,
  signature: Signature,
}

pub type IouMessage {
  subbit_id: SubbitId,
  amount: Amount,
}
