use aiken/collection/list
use cardano/address.{Address}
use cardano/transaction.{Input, Output, OutputReference, Transaction}
use subbit/types as t

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
