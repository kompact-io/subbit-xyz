use cardano/transaction.{OutputReference, Transaction}
use subbit/logic.{batch, main, mutual}
use subbit/types as t

validator placeholder {
  spend(
    datum: Option<t.Datum>,
    redeemer: t.Redeemer,
    own_oref: OutputReference,
    tx: Transaction,
  ) {
    expect Some(datum) = datum
    when redeemer is {
      t.Batch -> batch(datum.own_hash, own_oref, tx.inputs)
      t.Main(steps) -> main(datum, steps, own_oref, tx)
      t.Mutual -> mutual(datum, own_oref, tx)
    }
  }

  else(_) {
    fail
  }
}
