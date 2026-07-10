use cardano/transaction.{OutputReference, Transaction}
use subbit/spend.{do}
use subbit/types as t

validator konduit {
  spend(
    datum: Option<t.Datum>,
    redeemer: t.Redeemer,
    own_oref: OutputReference,
    tx: Transaction,
  ) {
    expect Some(datum) = datum
    do(datum, redeemer, own_oref, tx)
  }

  else(_) {
    fail
  }
}
