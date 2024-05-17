use aiken/builtin.{serialise_data}
use aiken/bytearray as ba
use aiken/hash.{blake2b_256}
use aiken/transaction.{OutputReference}
use subbit/constants.{tag_length}

pub fn auth_pref() -> ByteArray {
  #"000643b0"
}

pub fn vali_pref() -> ByteArray {
  #"000de140"
}

pub fn pref_len() -> Int {
  auth_pref() |> ba.length()
}

pub fn label() -> ByteArray {
  "subbit"
}

pub fn auth_label() {
  ba.concat(auth_pref(), label())
}

pub fn vali_label() {
  ba.concat(vali_pref(), label())
}

pub fn label_len() -> Int {
  vali_label() |> ba.length()
}

pub fn auth(tag: ByteArray) {
  ba.concat(auth_label(), tag)
}

pub fn vali(tag: ByteArray) {
  ba.concat(vali_label(), tag)
}

pub fn twins(tag: ByteArray) -> AList<ByteArray, Int> {
  [Pair(ba.concat(auth_label(), tag), 1), Pair(ba.concat(vali_label(), tag), 1)]
}

pub fn get_tag(an: ByteArray) {
  ba.drop(an, pref_len())
}

pub fn get_vali_tag(an: ByteArray) {
  expect vali_pref() == ba.take(an, label_len())
  get_tag(an)
}

/// Note that tn limits the combined length of label and tag.
pub fn mk_tag(oref: OutputReference) -> ByteArray {
  oref
    |> serialise_data()
    |> blake2b_256()
    |> ba.take(tag_length())
}
