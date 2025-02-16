//// output 

use aiken/collection/list
use aiken/crypto.{ScriptHash}
use aiken/fuzz.{and_then}
use aiken/interval.{Finite, Interval, IntervalBound}
use cardano/address.{Address, Script}
use cardano/assets
use cardano/transaction.{
  InlineDatum, Input, Output, OutputReference, Transaction, placeholder,
}
use mark/base.{
  ada, address_with_payment_credential, output_reference, script_hash,
  value_from, verification_key, verification_key_hash,
}
use mark/iou as iou_tests
use subbit/types as t

// Iou 

fn iou_test() {
  fuzz.one_of(
    [iou_tests.i0, iou_tests.i1, iou_tests.i2, iou_tests.i3, iou_tests.i4],
  )
}

// Hash 

fn subbit_id() {
  fuzz.bytearray_between(3, 32)
}

// Assets

const default_name = "subbit-rocks!"

fn by_hash() {
  let hash <- and_then(script_hash())
  fuzz.constant(t.ByHash(hash))
}

fn by_class() {
  let hash <- and_then(script_hash())
  let name <- and_then(fuzz.bytearray_between(0, 32))
  fuzz.constant(t.ByClass(hash, name))
}

fn currency() {
  fuzz.either3(fuzz.constant(t.Ada), by_hash(), by_class())
}

pub fn value(c: t.Currency) {
  when c is {
    t.Ada -> ada()
    t.ByHash(hash) -> value_from(hash, default_name)
    t.ByClass(hash, name) -> value_from(hash, name)
  }
}

fn value_currency(c: t.Currency, amt: t.Amount) {
  when c is {
    t.Ada -> assets.from_lovelace(amt)
    t.ByHash(hash) -> assets.from_asset(hash, default_name, amt)
    t.ByClass(hash, name) -> assets.from_asset(hash, name, amt)
  }
}

fn value_between(c: t.Currency, lb: Int, ub: Int) {
  let amt <- and_then(fuzz.int_between(lb, ub))
  let val =
    when c is {
      t.Ada -> assets.from_lovelace(amt)
      t.ByHash(hash) ->
        assets.from_lovelace(2_000_000)
          |> assets.add(hash, default_name, amt)
      t.ByClass(hash, name) ->
        assets.from_lovelace(2_000_000) |> assets.add(hash, name, amt)
    }
  fuzz.constant(val)
}

pub fn subbed() {
  fuzz.int_between(0, 1000000000000000000)
}

fn subbed_lt(amt: t.Amount) {
  fuzz.int_between(0, amt)
}

// Datum 

pub fn constants() {
  let subbit_id <- and_then(subbit_id())
  let currency <- and_then(currency())
  let iou_key <- and_then(verification_key())
  let consumer <- and_then(verification_key_hash())
  let provider <- and_then(verification_key_hash())
  let close_period <- and_then(fuzz.int_between(100000000, 1000000000000))
  let constants =
    t.Constants {
      subbit_id,
      currency,
      iou_key,
      consumer,
      provider,
      close_period,
    }
  fuzz.constant(constants)
}

fn constants_with(currency: t.Currency) {
  let subbit_id <- and_then(subbit_id())
  let iou_key <- and_then(verification_key())
  let consumer <- and_then(verification_key_hash())
  let provider <- and_then(verification_key_hash())
  let close_period <- and_then(fuzz.int_between(100000000, 1000000000000))
  let constants =
    t.Constants {
      subbit_id,
      currency,
      iou_key,
      consumer,
      provider,
      close_period,
    }
  fuzz.constant(constants)
}

fn opened() {
  let constants <- and_then(constants())
  let subbed <- and_then(fuzz.int_between(0, 1000000000000))
  fuzz.constant(t.Opened(constants, subbed))
}

fn opened_with(c: t.Currency) {
  let constants <- and_then(constants_with(c))
  let subbed <- and_then(fuzz.int_between(0, 1000000000000))
  fuzz.constant(t.Opened(constants, subbed))
}

fn closed() {
  let constants <- and_then(constants())
  let subbed <- and_then(fuzz.int_between(0, 1000000000000))
  let expire_at <- and_then(fuzz.int_between(7123223320, 123932993992991))
  fuzz.constant(t.Closed(constants, subbed, expire_at))
}

fn closed_with(c: t.Currency) {
  let constants <- and_then(constants_with(c))
  let subbed <- and_then(fuzz.int_between(0, 1000000000000))
  let expire_at <- and_then(fuzz.int_between(7123223320, 123932993992991))
  fuzz.constant(t.Closed(constants, subbed, expire_at))
}

fn settled() {
  let consumer <- and_then(verification_key_hash())
  fuzz.constant(t.Settled(consumer))
}

fn stage() {
  fuzz.either3(opened(), closed(), settled())
}

fn stage_with(c: t.Currency) {
  fuzz.either3(opened_with(c), closed_with(c), settled())
}

pub fn datum() {
  let own_hash <- and_then(script_hash())
  let stage <- and_then(stage())
  fuzz.constant(t.Datum { own_hash, stage })
}

fn datum_with(own_hash: ScriptHash, c: t.Currency) {
  let stage <- and_then(stage_with(c))
  fuzz.constant(t.Datum { own_hash, stage })
}

fn subbit_output(own_hash: ScriptHash) {
  let c <- and_then(currency())
  let address <- and_then(address_with_payment_credential(Script(own_hash)))
  let value <- and_then(value(c))
  let datum <- and_then(datum_with(own_hash, c))
  let reference_script = None
  fuzz.constant(
    Output { address, value, datum: InlineDatum(datum), reference_script },
  )
}

pub fn subbit_input(own_hash: ScriptHash) {
  let output_reference <- and_then(output_reference())
  let output <- and_then(subbit_output(own_hash))
  fuzz.constant(Input(output_reference, output))
}

pub fn tx_builder(
  size: Int,
) -> Fuzzer<(ScriptHash, OutputReference, Transaction)> {
  let tx = placeholder
  let own_oref <- and_then(output_reference())
  let own_hash <- and_then(script_hash())
  let own_hash: ScriptHash = own_hash
  let main_oref <- and_then(output_reference())
  let main_datum = t.Datum { own_hash, stage: t.Settled(own_hash) }
  let main_output =
    Output {
      address: Address {
        payment_credential: Script(own_hash),
        stake_credential: None,
      },
      value: assets.zero,
      datum: InlineDatum(main_datum),
      reference_script: None,
    }
  let main_input = Input { output_reference: main_oref, output: main_output }

  let inputs = []
  let inputs = inputs |> list.take(size) |> list.concat([main_input])

  let tx = Transaction { ..tx, inputs: inputs }
  fuzz.constant((own_hash, own_oref, tx))
}

pub type TestStep {
  datum: t.Datum,
  step: t.Step,
  own_oref: OutputReference,
  tx: Transaction,
}

pub fn add(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let step = t.StepCont(t.Add)

  let subbed <- and_then(subbed())
  let value_in <- and_then(value(c.currency))
  let datum = t.Datum(own_hash, t.Opened(c, subbed))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))

  let value_delta <- and_then(value(c.currency))
  let value_out = value_in |> assets.merge(value_delta)
  let output_out = Output(own_address, value_out, InlineDatum(datum_in), None)

  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      outputs: tx.outputs |> list.push(output_out),
      extra_signatories: tx.extra_signatories |> list.push(c.consumer),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

pub fn sub(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let iou_tests.IouTest { vkey: iou_key, subbit_id, amount, sig, .. } <-
    and_then(iou_test())
  let iou = t.Iou { amount, signature: sig }
  let step = t.StepCont(t.Sub(iou))

  let c = t.Constants { ..c, subbit_id: subbit_id, iou_key: iou_key }

  let subbed_in <- and_then(subbed_lt(amount))
  let subbed = amount - subbed_in
  let value_in <- and_then(value_between(c.currency, subbed, amount))
  let datum = t.Datum(own_hash, t.Opened(c, subbed_in))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))

  let value_out = value_in |> assets.merge(value_currency(c.currency, -subbed))
  let datum_out: Data = t.Datum(own_hash, t.Opened(c, amount))
  let output_out = Output(own_address, value_out, InlineDatum(datum_out), None)

  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      outputs: tx.outputs |> list.push(output_out),
      extra_signatories: tx.extra_signatories |> list.push(c.provider),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

pub fn close(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let step = t.StepCont(t.Close)

  let subbed_in <- and_then(subbed())
  let value_in <- and_then(value(c.currency))
  let datum = t.Datum(own_hash, t.Opened(c, subbed_in))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))

  let ub = 1000000
  let expire_at = ub + c.close_period + 1
  let datum_out: Data = t.Datum(own_hash, t.Closed(c, subbed_in, expire_at))
  let output_out = Output(own_address, value_in, InlineDatum(datum_out), None)

  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      outputs: tx.outputs |> list.push(output_out),
      extra_signatories: tx.extra_signatories |> list.push(c.consumer),
      validity_range: Interval(
        IntervalBound(Finite(0), False),
        IntervalBound(Finite(ub), False),
      ),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

pub fn settle(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let iou_tests.IouTest { vkey: iou_key, subbit_id, amount, sig, .. } <-
    and_then(iou_test())
  let iou = t.Iou { amount, signature: sig }
  let step = t.StepCont(t.Settle(iou))

  let c = t.Constants { ..c, subbit_id: subbit_id, iou_key: iou_key }

  let subbed_in <- and_then(subbed_lt(amount))
  let subbed = amount - subbed_in
  let value_in <- and_then(value_between(c.currency, subbed, amount))
  let expire_at <- and_then(fuzz.int_between(0, 1000000000000))
  let datum = t.Datum(own_hash, t.Closed(c, subbed_in, expire_at))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))

  let value_out = value_in |> assets.merge(value_currency(c.currency, -subbed))
  let datum_out: Data = t.Datum(own_hash, t.Settled(c.consumer))
  let output_out = Output(own_address, value_out, InlineDatum(datum_out), None)

  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      outputs: tx.outputs |> list.push(output_out),
      extra_signatories: tx.extra_signatories |> list.push(c.provider),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

pub fn end(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let step = t.StepEol(t.End)

  let value_in <- and_then(value(c.currency))
  let datum = t.Datum(own_hash, t.Settled(c.consumer))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))

  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      extra_signatories: tx.extra_signatories |> list.push(c.consumer),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

pub fn expire(own_hash: ScriptHash) {
  let own_address <- and_then(address_with_payment_credential(Script(own_hash)))
  let c <- and_then(constants())

  let step = t.StepEol(t.Expire)

  let lb <- and_then(fuzz.int_between(1, 100000000000))
  let expire_at <- and_then(fuzz.int_between(0, lb))

  let subbed_in <- and_then(subbed())
  let value_in <- and_then(value(c.currency))
  let datum = t.Datum(own_hash, t.Closed(c, subbed_in, expire_at))
  let datum_in: Data = datum
  let own_oref <- and_then(output_reference())
  let input_in =
    Input(own_oref, Output(own_address, value_in, InlineDatum(datum_in), None))
  let tx = placeholder
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs |> list.push(input_in),
      extra_signatories: tx.extra_signatories |> list.push(c.consumer),
      validity_range: Interval(
        IntervalBound(Finite(lb), False),
        IntervalBound(Finite(lb + 100000000), False),
      ),
    }
  fuzz.constant(TestStep { datum, step, own_oref, tx })
}

fn compose_tx(l: Transaction, r: Transaction) {
  Transaction {
    ..l,
    inputs: l.inputs |> list.concat(r.inputs),
    outputs: l.outputs |> list.concat(r.outputs),
    extra_signatories: l.extra_signatories |> list.concat(r.extra_signatories),
    validity_range: l.validity_range |> interval.intersection(r.validity_range),
  }
}

pub type ArgTriple {
  datum: t.Datum,
  step: t.Step,
  own_oref: OutputReference,
}

pub type MultiStep {
  triples: List<ArgTriple>,
  tx: Transaction,
}

fn test_steps_zero() {
  MultiStep([], placeholder)
}

pub fn stack(one: TestStep, many: MultiStep) {
  let TestStep { datum, step, own_oref, tx } = one
  MultiStep(
    many.triples |> list.concat([ArgTriple(datum, step, own_oref)]),
    many.tx |> compose_tx(tx),
  )
}

pub fn multi_subs(own_hash: ScriptHash, size: Int) {
  multi_subs_inner(own_hash, size, test_steps_zero())
}

fn multi_subs_inner(own_hash: ScriptHash, size: Int, x: MultiStep) {
  let sub <- and_then(sub(own_hash))
  let x = stack(sub, x)
  if size < 2 {
    fuzz.constant(x)
  } else {
    multi_subs_inner(own_hash, size - 1, x)
  }
}
