use cardano/transaction.{OutputReference, Transaction}
use subbit/spend/defer
use subbit/spend/main
use subbit/spend/mutual
use subbit/types as t

pub fn do(
  datum: t.Datum,
  redeemer: t.Redeemer,
  own_oref: OutputReference,
  tx: Transaction,
) {
  when redeemer is {
    t.Defer -> defer.do(datum, own_oref, tx)
    t.Main(steps) -> main.do(datum, steps, tx)
    t.Mutual -> mutual.do(datum, own_oref, tx)
  }
}
