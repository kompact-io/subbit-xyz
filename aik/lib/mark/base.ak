//// 

use aiken/collection/list
use aiken/crypto.{ScriptHash}
use aiken/fuzz.{and_then}
use cardano/address.{Address, Credential, Inline, Script, VerificationKey}
use cardano/assets
use cardano/transaction.{
  DatumHash, InlineDatum, Input, NoDatum, Output, OutputReference, Transaction,
  placeholder,
}

// Hash 

pub fn hash28() {
  fuzz.bytearray_between(28, 28)
}

pub fn hash32() {
  fuzz.bytearray_between(32, 32)
}

pub fn script_hash() -> Fuzzer<ScriptHash> {
  hash28()
}

pub fn verification_key_hash() -> Fuzzer<ScriptHash> {
  hash28()
}

pub fn transaction_id() {
  fuzz.bytearray_between(32, 32)
}

pub fn verification_key() {
  fuzz.bytearray_between(32, 32)
}

// Credential

pub fn credential() {
  let pred <- and_then(fuzz.bool())
  let hash <- and_then(script_hash())
  let cred =
    if pred {
      VerificationKey(hash)
    } else {
      Script(hash)
    }
  fuzz.constant(cred)
}

// Address

pub fn address() {
  let payment_credential <- and_then(credential())
  let is_stake <- and_then(fuzz.bool())
  let cred <- and_then(credential())
  let stake_credential =
    if is_stake {
      Some(Inline(cred))
    } else {
      None
    }
  fuzz.constant(Address(payment_credential, stake_credential))
}

pub fn address_with_payment_credential(payment_credential: Credential) {
  let is_stake <- and_then(fuzz.bool())
  let cred <- and_then(credential())
  let stake_credential =
    if is_stake {
      Some(Inline(cred))
    } else {
      None
    }
  fuzz.constant(Address(payment_credential, stake_credential))
}

// Assets

pub fn asset_class() -> Fuzzer<(ScriptHash, ByteArray)> {
  let script_hash <- and_then(script_hash())
  let name <- and_then(fuzz.bytearray_between(0, 32))
  fuzz.constant((script_hash, name))
}

pub fn asset_triple() -> Fuzzer<(ScriptHash, ByteArray, Int)> {
  let script_hash <- and_then(script_hash())
  let name <- and_then(fuzz.bytearray_between(0, 32))
  let amount <- and_then(fuzz.int_between(1, 100000000000000000))
  fuzz.constant((script_hash, name, amount))
}

pub fn ada() {
  let lovelace <- and_then(fuzz.int_between(1000000, 1000000000000))
  fuzz.constant(assets.from_lovelace(lovelace))
}

pub fn one_native_asset() {
  let lovelace <- and_then(fuzz.int_between(1000000, 4000000))
  let hash <- and_then(script_hash())
  let amount <- and_then(fuzz.int_between(1, 100000000000000000))
  let name = "my-little-token"
  fuzz.constant(
    assets.from_lovelace(lovelace)
      |> assets.add(hash, name, amount),
  )
}

pub fn value() {
  let ada <- and_then(ada())
  let nas <- and_then(fuzz.list(asset_triple()))
  let tot =
    nas |> list.foldl(ada, fn(x, y) { y |> assets.add(x.1st, x.2nd, x.3rd) })
  fuzz.constant(tot)
}

pub fn value_from(hash: ScriptHash, name: ByteArray) {
  let ada <- and_then(ada())
  let amount <- and_then(fuzz.int_between(1, 100000000000000000))
  let tot = ada |> assets.add(hash, name, amount)
  fuzz.constant(tot)
}

// Datum

pub fn datum() {
  let hash <- and_then(hash32())
  fuzz.one_of([NoDatum, InlineDatum(hash), DatumHash(hash)])
}

// Reference script 

pub fn reference_script() {
  let hash <- and_then(hash32())
  fuzz.one_of([None, Some(hash)])
}

// Output 

pub fn output() {
  let address <- and_then(address())
  let value <- and_then(value())
  let datum <- and_then(datum())
  let reference_script <- and_then(reference_script())
  fuzz.constant(Output(address, value, datum, reference_script))
}

// Input 

pub fn output_index() {
  fuzz.int_between(0, 513)
}

pub fn output_reference() -> Fuzzer<OutputReference> {
  let transaction_id <- and_then(transaction_id())
  let output_index <- and_then(output_index())
  fuzz.constant(OutputReference { transaction_id, output_index })
}

pub fn input() {
  let output_reference <- and_then(output_reference())
  let output <- and_then(output())
  fuzz.constant(Input(output_reference, output))
}

pub fn inputs() {
  fuzz.list(input())
}

pub fn transaction() -> Fuzzer<Transaction> {
  fuzz.constant(placeholder)
}

pub fn extra_signers() {
  fuzz.list_between(verification_key_hash(), 2, 5)
}
