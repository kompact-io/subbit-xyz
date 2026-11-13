use aiken/collection/list
use cardano/address.{Address, VerificationKey}
use cardano/assets
use cardano/transaction.{
  Input, NoDatum, Output, OutputReference, Transaction, placeholder,
}
use subbit/fixtures as fx
use subbit/types as t

fn wallet_input(oref: OutputReference) -> Input {
  Input {
    output_reference: oref,
    output: Output {
      address: Address {
        payment_credential: VerificationKey(#"aa"),
        stake_credential: None,
      },
      value: assets.from_lovelace(5),
      datum: NoDatum,
      reference_script: None,
    },
  }
}

/// A channel can be spent by mutual consent with no further verification on the
/// channel itself. In such cases, it is spent with the `Mutual` redeemer. To prevent
/// exploits, and by the way `Defer` and `Main` interplay, we prevent any other
/// channels spent alongside a `Mutual` spend.
pub fn do(datum: t.Datum, own_oref: OutputReference, tx: Transaction) {
  let (_, constants, _) = datum
  let (_tag, _currency, _iou_key, consumer, provider, _close_period) = constants
  let Transaction { inputs, extra_signatories, .. } = tx
  trace @"mutual.0": @"Both participants sign"
  expect extra_signatories |> list.has(consumer)
  expect extra_signatories |> list.has(provider)
  expect Some(own_input) = inputs
    |> list.find(fn(i) { i.output_reference == own_oref })
  let own_cred = own_input.output.address.payment_credential
  trace @"mutual.1": @"Is solo channel input"
  1 == (
    inputs
      |> list.count(fn(i) { i.output.address.payment_credential == own_cred })
  )
}

test mutual_succeeds_when_both_sign_solo() {
  let stage = t.Opened { subbed: 0 }
  let own_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let tx = Transaction {
    ..placeholder,
    inputs: [own_in],
    extra_signatories: [fx.consumer_vkh, fx.provider_vkh],
  }
  do(fx.datum(stage), fx.oref(0), tx)
}

test mutual_fails_when_consumer_missing() fail {
  let stage = t.Opened { subbed: 0 }
  let own_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let tx = Transaction {
    ..placeholder,
    inputs: [own_in],
    extra_signatories: [fx.provider_vkh],
  }
  expect _ok = do(fx.datum(stage), fx.oref(0), tx)
  True
}

test mutual_fails_when_provider_missing() fail {
  let stage = t.Opened { subbed: 0 }
  let own_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let tx = Transaction {
    ..placeholder,
    inputs: [own_in],
    extra_signatories: [fx.consumer_vkh],
  }
  expect _ok = do(fx.datum(stage), fx.oref(0), tx)
  True
}

test mutual_fails_when_own_input_missing() fail {
  let stage = t.Opened { subbed: 0 }
  let tx = Transaction {
    ..placeholder,
    inputs: [],
    extra_signatories: [fx.consumer_vkh, fx.provider_vkh],
  }
  expect _ok = do(fx.datum(stage), fx.oref(0), tx)
  True
}

test mutual_with_additional_input_fails() {
  let stage = t.Opened { subbed: 0 }
  let own_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let other_own_in = fx.own_input(fx.oref(1), stage, assets.from_lovelace(10))
  let tx = Transaction {
    ..placeholder,
    inputs: [own_in, other_own_in],
    extra_signatories: [fx.consumer_vkh, fx.provider_vkh],
  }
  !do(fx.datum(stage), fx.oref(0), tx)
}

test mutual_succeeds_alongside_unrelated_wallet_input() {
  let stage = t.Opened { subbed: 0 }
  let own_in = fx.own_input(fx.oref(0), stage, assets.from_lovelace(10))
  let tx = Transaction {
    ..placeholder,
    inputs: [own_in, wallet_input(fx.oref(1))],
    extra_signatories: [fx.consumer_vkh, fx.provider_vkh],
  }
  do(fx.datum(stage), fx.oref(0), tx)
}
