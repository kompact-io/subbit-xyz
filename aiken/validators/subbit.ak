use cardano/assets
use cardano/transaction.{OutputReference, Transaction, placeholder}
use subbit/fixtures as fx
use subbit/spend.{do}
use subbit/types as t

validator subbit {
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

test subbit_spend_add_via_main() {
  let stage = t.Opened { subbed: 0 }
  let tx = Transaction {
    ..placeholder,
    inputs: [fx.own_input(fx.oref(0), stage, assets.from_lovelace(100))],
    outputs: [fx.own_output(stage, assets.from_lovelace(150))],
    extra_signatories: [fx.consumer_vkh],
  }
  subbit.spend(
    Some(fx.datum(stage)),
    t.Main([t.StepCont(t.Add)]),
    fx.oref(0),
    tx,
  )
}

test subbit_spend_mutual() {
  let stage = t.Opened { subbed: 0 }
  let tx = Transaction {
    ..placeholder,
    inputs: [fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))],
    extra_signatories: [fx.consumer_vkh, fx.provider_vkh],
  }
  subbit.spend(Some(fx.datum(stage)), t.Mutual, fx.oref(0), tx)
}

test subbit_spend_defer() {
  let stage = t.Opened { subbed: 0 }
  let main_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let deferred_in = fx.own_input(fx.oref(1), stage, assets.from_lovelace(10))
  let tx = Transaction { ..placeholder, inputs: [main_in, deferred_in] }
  subbit.spend(Some(fx.datum(stage)), t.Defer, fx.oref(1), tx)
}

test subbit_spend_fails_without_datum() fail {
  expect _ok = subbit.spend(None, t.Mutual, fx.oref(0), placeholder)
  True
}
