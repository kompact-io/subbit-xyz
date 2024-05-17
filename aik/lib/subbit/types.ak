use aiken/transaction.{OutputReference}

pub type Count =
  Int

pub type PubKey =
  ByteArray

pub type Dat {
  Open(OpenParams)
  Closed(ClosedParams)
  Done
}

pub type OpenParams {
  consumer: PubKey,
  provider: PubKey,
  last: Int,
  count: Count,
}

pub type ClosedParams {
  consumer: PubKey,
  provider: PubKey,
  deadline: Int,
  count: Count,
}

pub type Red2 {
  Init(OutputReference)
  Burn
}

pub type ContIdx =
  Int

pub type AuthIdx =
  Int

pub type Proof =
  ByteArray

pub type Red3 {
  Add(ContIdx, AuthIdx)
  Sub(ContIdx, Count, Proof)
  Close(ContIdx, AuthIdx)
  Drain
}

pub type SubParams {
  cont_idx: Int,
  amt: Int,
  proof: ByteArray,
}
