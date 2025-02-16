use aiken/collection/list
use aiken/fuzz.{and_then}
use cardano/address.{Script}
use cardano/transaction.{InlineDatum, Input, Output, Transaction, placeholder}
use mark/base.{address_with_payment_credential, output_reference, script_hash}
use mark/gen.{constants, subbed, value}
use subbit/logic.{mutual}
use subbit/types as t

pub fn gen_args() {
  let own_hash <- and_then(script_hash())
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())
  let subbed_in <- and_then(subbed())
  let value_in <- and_then(value(c.currency))
  let datum = t.Datum(own_hash, t.Opened(c, subbed_in))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))
  let tx = placeholder
  let pred <- and_then(fuzz.bool())
  let extra_signatories =
    if pred {
      [c.consumer, c.provider]
    } else {
      [c.provider, c.consumer]
    }
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      extra_signatories: extra_signatories,
    }
  fuzz.constant((datum, own_oref, tx))
}

test test_mutual((datum, own_oref, tx) via gen_args()) {
  mutual(datum, own_oref, tx)
}

test fail_mutual((datum, own_oref, tx) via gen_args()) fail {
  expect Some(partner) = tx.extra_signatories |> list.at(0)
  mutual(datum, own_oref, Transaction { ..tx, extra_signatories: [partner] })
}
