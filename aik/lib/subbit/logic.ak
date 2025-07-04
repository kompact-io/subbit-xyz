use aiken/cbor
use aiken/collection/dict
use aiken/collection/list
use aiken/crypto.{
  ScriptHash, VerificationKey, VerificationKeyHash, verify_ed25519_signature,
}
use aiken/interval.{Finite, Interval, IntervalBound}
use cardano/address.{Address, Credential, Script}
use cardano/assets.{Value}
use cardano/transaction.{
  Datum, InlineDatum, Input, Output, OutputReference, Transaction,
}
use subbit/types as t

type Signers =
  List<VerificationKeyHash>

type Bound =
  IntervalBound<Int>

fn unbound(b: Bound) {
  expect IntervalBound { bound_type: Finite(b), .. } = b
  b
}

pub fn verify_iou(key: VerificationKey, tag: t.Tag, iou: t.Iou) {
  let t.Iou { amount, signature } = iou
  let message: Data = t.IouMessage(tag, amount)
  verify_ed25519_signature(key, cbor.serialise(message), signature)
}

/// ### Batch
/// 
/// In a standard tx, all but one of the script inputs is spent with `Batch`. More
/// precisely, the lexicographical first validator input in the tx inputs must have
/// redeemer `Main`, and all the rest are spent with `Batch`.
/// 
/// The logic when the redeemer is `Batch`:
type DatumReduce {
  own_hash: ScriptHash,
  other: Data,
}

fn get_own_hash(datum: Datum) {
  expect InlineDatum(inline): Datum = datum
  expect DatumReduce { own_hash, .. }: DatumReduce = inline
  own_hash
}

pub fn batch(
  own_hash: ScriptHash,
  own_oref: OutputReference,
  inputs: List<Input>,
) {
  trace @"batch.0": @"First script input `main_input` is not own input"
  trace @"batch.1": @"`main_input` datum has matching `own_hash`"
  let main_oref = first_oref_with_cred(Script(own_hash), inputs)
  own_oref != main_oref
}

pub fn first_oref_with_cred(
  cred: Credential,
  inputs: List<Input>,
) -> OutputReference {
  when inputs is {
    [input, ..inputs] -> {
      let Input {
        output_reference: oref,
        output: Output { address, datum, .. },
      } = input
      if address.payment_credential == cred {
        expect Script(get_own_hash(datum)) == cred
        oref
      } else {
        first_oref_with_cred(cred, inputs)
      }
    }
    _ -> fail @"impossible"
  }
}

fn yield_own_input(
  own_oref: OutputReference,
  inputs: List<Input>,
) -> (Input, List<Input>) {
  when inputs is {
    [input, ..inputs] -> {
      let Input { output_reference, .. } = input
      if own_oref == output_reference {
        (input, inputs)
      } else {
        yield_own_input(own_oref, inputs)
      }
    }
    _ -> fail @"impossible"
  }
}

/// ### Main
/// 
/// In a standard tx, the "main" invocation does all the verification steps for all
/// subbits.
/// 
/// The logic when the redeemer is `Main(steps)`
/// 
pub fn main(
  datum: t.Datum,
  steps: t.Steps,
  own_oref: OutputReference,
  tx: Transaction,
) {
  trace @"main.0": @"Extract `bounds`, and `extra_signatories`,"
  let Transaction { inputs, outputs, validity_range, extra_signatories, .. } =
    tx
  trace @"main.1": @"Find own input"
  let (input, inputs) = yield_own_input(own_oref, inputs)
  let own_hash = datum.own_hash
  expect Script(datum.own_hash) == input.output.address.payment_credential
  trace @"main.2": @"`recur` over `steps`, `inputs` and `outputs`"
  let inputs =
    recur(
      own_hash,
      extra_signatories,
      validity_range,
      steps,
      inputs |> list.push(input),
      outputs,
    )
  trace @"main.3": @"No more subbits"
  let own_cred = Script(own_hash)
  !(inputs
    |> list.any(fn(i) { i.output.address.payment_credential == own_cred }))
}

/// ### Recur
/// 
/// The recur function exhausts the list of steps that need to be verified. Each
/// step verification requires some context: certainly the subbit input, and if a
/// continuing step then the continuing output, as well as the tx signers list and
/// perhaps the tx validity range. The recur function organises this.
/// 
fn recur(
  own_hash: ScriptHash,
  signers: Signers,
  bounds: Interval<Int>,
  steps: t.Steps,
  inputs: List<Input>,
  outputs: List<Output>,
) {
  when steps is {
    [step, ..steps] -> {
      trace @"recur.0": @"yield subbit in else fail"
      let (address_in, value_in, stage_in, inputs) = yield_in(own_hash, inputs)
      trace @"recur.1": @"yield step else return inputs"
      when step is {
        t.StepCont(step) -> {
          trace @"recur.2": @"if step is continuing"
          let currency = get_currency(stage_in)
          let funds_in = amount_in(currency, value_in)
          trace @"recur.2.0": @"yield continuing output else fail"
          let (funds_out, stage_out, outputs) =
            yield_out(address_in, currency, outputs)
          trace @"recur.2.1": @"verify step"
          expect
            when step is {
              t.Add -> do_add(signers, funds_in, stage_in, funds_out, stage_out)
              t.Sub(iou) ->
                do_sub(signers, funds_in, stage_in, funds_out, stage_out, iou)
              t.Close ->
                do_close(
                  signers,
                  bounds.upper_bound,
                  funds_in,
                  stage_in,
                  funds_out,
                  stage_out,
                )
              t.Settle(iou) ->
                do_settle(
                  signers,
                  funds_in,
                  stage_in,
                  funds_out,
                  stage_out,
                  iou,
                )
            }
          recur(own_hash, signers, bounds, steps, inputs, outputs)
        }
        t.StepEol(step) -> {
          trace @"recur.3": @"else"
          trace @"recur.3": @"verify step"
          expect
            when step is {
              t.End -> do_end(signers, stage_in)
              t.Expire -> do_expire(signers, bounds.lower_bound, stage_in)
            }
          recur(own_hash, signers, bounds, steps, inputs, outputs)
        }
      }
    }
    [] -> inputs
  }
}

/// ### Mutual
/// 
/// A subbit can be spent by mutual consent with no further verification on the
/// subbit itself. In such cases, it is spent with the `Mutual` redeemer. To prevent
/// exploits, and by the way `Batch` and `Main` interplay, we prevent any other
/// subbits spent alongside a `Mutual` spend.
pub fn mutual(datum: t.Datum, own_oref: OutputReference, tx: Transaction) {
  let (consumer, provider) =
    when datum.stage is {
      t.Opened(t.Constants { consumer, provider, .. }, _) ->
        (consumer, provider)
      t.Closed(t.Constants { consumer, provider, .. }, _, _) ->
        (consumer, provider)
      _ -> fail @"unsupported"
    }
  let Transaction { inputs, extra_signatories, .. } = tx
  trace @"mutual.0": @"Both participants sign"
  expect and {
      extra_signatories |> list.has(consumer),
      extra_signatories |> list.has(provider),
    }
  expect Some(own_input) =
    inputs |> list.find(fn(i) { i.output_reference == own_oref })
  let own_cred = own_input.output.address.payment_credential
  trace @"mutual.1": @"Is solo subbit input"
  1 == (
    inputs
      |> list.count(fn(i) { i.output.address.payment_credential == own_cred })
  )
}

/// ### No more
/// 
/// In the case of batch txs, there cannot be any subbit spent without a
/// corresponding step. In the case of mutual txs, there is only a single, solo
/// subbit input. Once the list of steps has been exhausted or the solo subbit input
/// has been found, it remains to verify there are no more subbit inputs.
fn no_more(own_hash: ScriptHash, inputs: List<Input>) {
  when inputs is {
    [] -> True
    [input, ..inputs] -> {
      expect Script(own_hash) != input.output.address.payment_credential
      no_more(own_hash, inputs)
    }
  }
}

/// #### Yield subbit in
/// 
/// We define a function that "yields" the next subbit input. A subbit input is
/// identified by its payment credential. The rest of the input is then parsed to
/// check correctness. 
fn yield_in(
  own_hash: ScriptHash,
  inputs: List<Input>,
) -> (Address, Value, t.Stage, List<Input>) {
  when inputs is {
    [input, ..inputs] -> {
      let Output { address, value, datum, .. } = input.output
      if address.payment_credential == Script(own_hash) {
        (address, value, coerce_datum(datum).stage, inputs)
      } else {
        yield_in(own_hash, inputs)
      }
    }
    [] -> fail @"expect subbit in"
  }
}

/// #### Amount in
/// 
/// A subbit supports mono-assets. See [funds](../adrs/funds.md). The subbit
/// currency is recorded in the constants. On a subbit input, we are permissive
/// about the value, ie any unanticipated native assets are ignored.
fn amount_in(currency: t.Currency, value: Value) -> t.Amount {
  when currency is {
    t.Ada -> value |> assets.lovelace_of
    t.ByHash(hash) -> {
      expect [Pair(_name, amount), ..] =
        value |> assets.tokens(hash) |> dict.to_pairs()
      amount
    }
    t.ByClass(hash, name) -> value |> assets.quantity_of(hash, name)
  }
}

/// #### Amount out
/// 
/// Continuing outputs value is handled in a stricter manner than subbit inputs. The
/// amount out function is similar to the amount in function, but checks the value
/// contains no additional assets. This is to prevent token spamming attacks.
fn amount_out(currency: t.Currency, value: Value) -> t.Amount {
  let flat = value |> assets.flatten
  when flat is {
    [(_, _, amt)] ->
      when currency is {
        t.Ada -> amt
        _ -> 0
      }
    [_, (hash_, name_, amt)] ->
      when currency is {
        t.ByHash(hash) -> {
          expect hash == hash_
          amt
        }
        t.ByClass(hash, name) -> {
          expect and {
              hash == hash_,
              name == name_,
            }
          amt
        }
        t.Ada -> fail @"only ada"
      }
    _ -> fail @"<2 assets"
  }
}

/// #### Yield continuing output
/// 
/// For any non-terminal step there is a continuing output. The checks required are
/// subtly different to that of subbit in.
/// 
/// Recall that the continuing output must match address, not just payment
/// credential. Since the currency should be unchanged, we use the currency value
/// from the subbit input. The step logic will verify all the entire constant value
/// is unchanged, where relevant.
fn yield_out(
  address_: Address,
  currency: t.Currency,
  outputs: List<Output>,
) -> (t.Amount, t.Stage, List<Output>) {
  when outputs is {
    [output, ..outputs] -> {
      let Output { address, value, datum, reference_script } = output
      if address_ == address {
        let t.Datum { own_hash, stage } = coerce_datum(datum)
        expect and {
            Script(own_hash) == address.payment_credential,
            reference_script == None,
          }
        let amount = amount_out(currency, value)
        (amount, stage, outputs)
      } else {
        yield_out(address_, currency, outputs)
      }
    }
    _ -> fail @"expected output"
  }
}

pub fn do_add(
  signers: Signers,
  funds_in: t.Amount,
  stage_in: t.Stage,
  funds_out: t.Amount,
  stage_out: t.Stage,
) -> Bool {
  trace @"add.0": @"Stage in is opened : `stage_in = Opened(constants, subbed)`"
  expect t.Opened(constants, _) = stage_in
  trace @"add.1": @"Consumer has signed"
  expect signers |> list.has(constants.consumer)
  trace @"add.2": @"Stage out is equal to stage in `stage_out == stage_in`"
  expect stage_in == stage_out
  trace @"add.3": @"Funds increased `funds_in` < `funds_out`"
  funds_in < funds_out
}

pub fn do_sub(
  signers: Signers,
  funds_in: t.Amount,
  stage_in: t.Stage,
  funds_out: t.Amount,
  stage_out: t.Stage,
  iou: t.Iou,
) -> Bool {
  trace @"sub.0": @"Stage in is opened : `stage_in = Opened(constants, subbed_in)`"
  expect t.Opened(constants, subbed_in) = stage_in
  let t.Constants { tag, iou_key, provider, .. } = constants
  trace @"sub.1": @"Provider has signed"
  expect signers |> list.has(provider)
  trace @"sub.2": @"Stage out is opened : `stage_out = Opened(constants_, subbed_out)`"
  expect t.Opened(constants_, subbed_out) = stage_out
  expect constants == constants_
  trace @"sub.3": @"Funds decrease by `subbed = funds_in - funds_out`"
  let subbed = funds_in - funds_out
  trace @"sub.4": @"Subbed amount is correct `subbed_out == subbed_in + subbed`"
  expect subbed_out == subbed_in + subbed
  trace @"sub.5": @"iou has valid signature"
  expect verify_iou(iou_key, tag, iou)
  trace @"sub.6": @"iou permits subbed out `iou.amount >= subbed_out`"
  iou.amount >= subbed_out
}

pub fn do_close(
  signers: Signers,
  upper_bound: Bound,
  funds_in: t.Amount,
  stage_in: t.Stage,
  funds_out: t.Amount,
  stage_out: t.Stage,
) -> Bool {
  trace @"close.0": @"Stage in is opened : `stage_in = Opened(constants, subbed)`"
  expect t.Opened(constants, subbed) = stage_in
  let t.Constants { consumer, close_period, .. } = constants
  trace @"close.1": @"Consumer has signed"
  expect signers |> list.has(consumer)
  trace @"close.2": @"Stage out is closed : `stage_out = Closed(constants_, subbed_, expire_at)`"
  expect t.Closed(constants_, subbed_, expire_at) = stage_out
  expect and {
      constants == constants_,
      subbed == subbed_,
    }
  trace @"close.3": @"Funds unchanged"
  expect funds_in == funds_out
  trace @"close.4": @"Expire at respects the close period : `expire_at >= upper_bound + close_period`"
  expire_at >= unbound(upper_bound) + close_period
}

pub fn do_settle(
  signers: Signers,
  funds_in: t.Amount,
  stage_in: t.Stage,
  funds_out: t.Amount,
  stage_out: t.Stage,
  iou: t.Iou,
) -> Bool {
  trace @"settle.0": @"Stage in is Closed : `stage_in = Closed(constants, subbed, _)`"
  expect t.Closed(constants, subbed_in, _) = stage_in
  let t.Constants { tag, iou_key, consumer, provider, .. } = constants
  trace @"settle.1": @"Provider has signed"
  expect signers |> list.has(provider)
  trace @"settle.2": @"Stage out is opened : `stage_out = Settled(constants.consumer)`"
  expect t.Settled(consumer_) = stage_out
  expect consumer == consumer_
  trace @"settle.3": @"Funds decrease by `subbed = funds_in - funds_out`"
  let subbed = funds_in - funds_out
  trace @"settle.4": @"Subbed amount is correct `subbed_out == subbed_in + subbed`"
  let subbed_out = subbed_in + subbed
  trace @"settle.5": @"iou has valid signature"
  expect verify_iou(iou_key, tag, iou)
  trace @"settle.6": @"iou permits subbed out `iou.amount >= subbed_out`"
  iou.amount >= subbed_out
}

pub fn do_end(signers: Signers, stage_in: t.Stage) -> Bool {
  trace @"end.0": @"Stage in is settled : `stage_in = Settled(consumer)`"
  expect t.Settled(consumer) = stage_in
  trace @"end.1": @"Consumer has signed"
  signers |> list.has(consumer)
}

pub fn do_expire(
  signers: Signers,
  lower_bound: Bound,
  stage_in: t.Stage,
) -> Bool {
  trace @"expire.0": @"Stage in is Closed : `stage_in = Closed(constants, _, expired_at)`"
  expect t.Closed(constants, _, expire_at) = stage_in
  let t.Constants { consumer, .. } = constants
  trace @"expire.1": @"Consumer has signed"
  expect signers |> list.has(consumer)
  trace @"expire.2": @"Expire at respects the close period : `expire_at <= lower_bound`"
  expire_at <= unbound(lower_bound)
}

fn coerce_datum(d: Datum) -> t.Datum {
  expect InlineDatum(data) = d
  expect d: t.Datum = data
  d
}

fn get_currency(stage: t.Stage) -> t.Currency {
  when stage is {
    t.Opened(c, _) -> c.currency
    t.Closed(c, _, _) -> c.currency
    _ -> fail @"currency unknown"
  }
}
