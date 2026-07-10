use cardano/address.{Address, Credential, Script, StakeCredential}
use cardano/assets.{Value}
use cardano/transaction.{Datum, InlineDatum, Input, Output}
use subbit/types as t

/// #### Yield channel in
/// 
/// We define a function that "yields" the next channel input. A channel input is
/// identified by its payment credential. The rest of the input is then parsed to
/// check correctness. 
pub type YeildIn<result> =
  fn(Option<StakeCredential>, Value, t.Constants, t.Stage, List<Input>) ->
    result

pub fn yield_in(
  own_cred: Credential,
  inputs: List<Input>,
  return: YeildIn<result>,
) -> result {
  expect [input, ..inputs] = inputs
  let Output { address, value, datum, .. } = input.output
  let Address { payment_credential, stake_credential } = address
  if payment_credential == own_cred {
    let (own_hash, constants, stage) = coerce_datum(datum)
    expect Script(own_hash) == own_cred
    return(stake_credential, value, constants, stage, inputs)
  } else {
    yield_in(own_cred, inputs, return)
  }
}

/// #### Yield continuing output
/// 
/// For any non-terminal step there is a continuing output. The checks required are
/// subtly different to that of channel in.
/// 
/// Recall that the continuing output must match address, not just payment
/// credential. Since the currency should be unchanged, we use the currency value
/// from the channel input. The step logic will verify all the entire constant value
/// is unchanged, where relevant.
pub type YeildOut<result> =
  fn(Value, t.Stage, List<Output>) -> result

pub fn yield_out(
  own_cred: Credential,
  delegation: Option<StakeCredential>,
  constants: t.Constants,
  outputs: List<Output>,
  return: YeildOut<result>,
) -> result {
  expect [output, ..outputs] = outputs
  let Output { address, value, datum, reference_script } = output
  let Address { payment_credential, stake_credential } = address
  if payment_credential == own_cred {
    expect stake_credential == delegation
    let (own_hash_, constants_, stage) = coerce_datum(datum)
    expect Script(own_hash_) == own_cred
    expect constants_ == constants
    expect reference_script == None
    return(value, stage, outputs)
  } else {
    yield_out(own_cred, delegation, constants, outputs, return)
  }
}

fn coerce_datum(d: Datum) -> t.Datum {
  expect InlineDatum(data) = d
  expect d: t.Datum = data
  d
}
