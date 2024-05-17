//// 

use aiken/builtin
use aiken/bytearray as ba
use aiken/dict
use aiken/hash
use aiken/interval
use aiken/list
use aiken/transaction.{
  InlineDatum, Input, Mint, Output, ScriptContext, Spend, Transaction,
}
use aiken/transaction/credential.{Address, ScriptCredential}
use aiken/transaction/value
use subbit/tokens
use subbit/types as t

fn mk_message(tag: ByteArray, count: t.Count) {
  tag
    |> ba.concat(integer_to_bytearray(False, 0, count))
    |> hash.blake2b_256
}

fn verify(vkey: t.PubKey, message: ByteArray, proof: ByteArray) {
  builtin.verify_ed25519_signature(vkey, message, proof)
}

validator {
  fn two(red: t.Red2, ctx: ScriptContext) {
    trace @"Two params: Purpose is `Mint(own_hash)`"
    expect ScriptContext {
      transaction: Transaction { inputs, mint, outputs, .. },
      purpose: Mint(own_hash),
    } = ctx
    let own_mint =
      mint
        |> value.from_minted_value
        |> value.tokens(own_hash)
        |> dict.to_list()
    when red is {
      t.Init(seed) -> {
        trace @"When red is `Init(seed)`:"
        trace @"1. `seed` is spent"
        expect inputs |> list.any(fn(i) { i.output_reference == seed })
        trace @"2. Own mint value is twin pair with `tag(seed)`"
        let tag = tokens.mk_tag(seed)
        expect tokens.twins(tag) == own_mint
        trace @"3. Output script token to own (payment) address"
        outputs
          |> list.any(
              fn(o) {
                let Output { address, value, .. } = o
                when value |> value.flatten is {
                  [_, (own_hash_, name, _)] ->
                    if own_hash_ == own_hash && name == tokens.vali(tag) {
                      expect
                        ScriptCredential(own_hash) == address.payment_credential
                      True
                    } else {
                      False
                    }
                  _ -> False
                }
              },
            )
      }
      t.Burn -> {
        trace @"When red is `Burn`:"
        trace @"1. Own mint value is negative"
        own_mint |> list.all(fn(tup) { tup.2nd < 0 })
      }
    }
  }

  fn three(dat: t.Dat, red: t.Red3, ctx: ScriptContext) {
    trace @"Three params: Purpose is `Spend(own_oref)`."
    expect ScriptContext {
      transaction: Transaction {
        inputs,
        mint,
        outputs,
        extra_signatories,
        validity_range,
        ..
      },
      purpose: Spend(own_oref),
    } = ctx
    expect Some(own_input) =
      inputs |> list.find(fn(i) { i.output_reference == own_oref })
    let Input { output: Output { address: own_addr, value: own_value, .. }, .. } =
      own_input
    let Address { payment_credential: own_cred, .. } = own_addr
    expect ScriptCredential(own_hash) = own_cred
    expect [(_, _, own_ada), (own_hash_, own_vali_name, _)] =
      own_value |> value.flatten()
    // let own_mint = mint |> value.from_minted_value |> value.tokens(own_hash) |> dict.to_list()
    expect own_hash == own_hash_
    let own_tag = tokens.get_vali_tag(own_vali_name)
    when (dat, red) is {
      (t.Open(_), t.Add(cont_idx, auth_idx)) -> {
        trace @"1. `Open(open_params), Add(cont_idx, auth_idx) `"
        expect Some(Output {
          address: cont_addr,
          value: cont_value,
          datum: InlineDatum(cont_idat),
          reference_script: None,
        }) = outputs |> list.at(cont_idx)
        trace @"    1. Cont address is: "
        trace @"        1. Own address, or "
        trace @"        2. Own payment credential, and auth token in `auth_idx` output"
        expect or {
            cont_addr == own_addr,
            {
              let Address { payment_credential: cont_cred, .. } = cont_addr
              expect Some(Output { value: auth_value, .. }) =
                outputs |> list.at(auth_idx)
              and {
                cont_cred == own_cred,
                (
                  auth_value
                    |> value.quantity_of(own_hash, tokens.auth(own_tag))
                ) == 1,
              }
            },
          }
        trace @"    2. `diff >= 0`"
        expect [(_, _, cont_ada), (own_hash_, name, _)] =
          cont_value |> value.flatten()
        expect and {
            own_hash_ == own_hash,
            name == own_vali_name,
          }
        let diff = cont_ada - own_ada
        expect diff >= 0
        trace @"    3. Cont datum is own datum"
        let dat_as_data: Data = dat
        cont_idat == dat_as_data
      }
      (
        t.Open(t.OpenParams { consumer, provider, last, count: own_count }),
        t.Sub(cont_idx, count, proof),
      ) -> {
        trace @"1. `Open(open_params), Sub(cont_idx, count, proof)`"
        expect Some(Output {
          address: cont_addr,
          value: cont_value,
          datum: InlineDatum(cont_idat),
          reference_script: None,
        }) = outputs |> list.at(cont_idx)
        trace @"    1. Cont address is own address"
        expect cont_addr == own_addr
        trace @"    2. `count - open_params.count >= (- diff)`"
        expect [(_, _, cont_ada), (own_hash_, name, _)] =
          cont_value |> value.flatten()
        expect and {
            own_hash_ == own_hash,
            name == own_vali_name,
          }
        let diff = cont_ada - own_ada
        expect count - own_count >= -diff
        trace @"    3. Cont datum is `Open { ..open_params , count}`"
        expect t.Open(cont_open_params): t.Dat = cont_idat
        expect
          t.OpenParams { consumer, provider, last, count } == cont_open_params
        trace @"    4. Provider has signed tx"
        expect extra_signatories |> list.any(fn(s) { s == provider })
        trace @"    5. Verify proof `verify(consumer, count, proof)`"
        verify(consumer, mk_message(own_tag, count), proof)
      }
      (
        t.Open(t.OpenParams { consumer, provider, last, count }),
        t.Close(cont_idx, auth_idx),
      ) -> {
        trace @"1. `Open(OpenParams { consumer, provider, last, count,}), Close(cont_idx, auth_idx)`"
        expect Some(Output {
          address: Address { payment_credential: cont_cred, .. },
          value: cont_value,
          datum: InlineDatum(cont_idat),
          reference_script: None,
        }) = outputs |> list.at(cont_idx)
        trace @"    1. Cont payment cred is own payment cred"
        expect cont_cred == own_cred
        trace @"    2. `diff >= 0`"
        expect [(_, _, cont_ada), (own_hash_, name, _)] =
          cont_value |> value.flatten()
        expect and {
            own_hash_ == own_hash,
            name == own_vali_name,
          }
        let diff = cont_ada - own_ada
        expect diff >= 0
        trace @"    3. Cont datum is `Closed { consumer, provider, deadline, count}`"
        expect t.Closed(t.ClosedParams {
          consumer: cont_consumer,
          provider: cont_provider,
          deadline,
          count: cont_count,
        }): t.Dat = cont_idat
        expect and {
            cont_consumer == consumer,
            cont_provider == provider,
            cont_count == count,
          }
        trace @"    4. Auth token in `auth_idx` output"
        expect Some(Output { value: auth_value, .. }) =
          outputs |> list.at(auth_idx)
        expect
          ( auth_value |> value.quantity_of(own_hash, tokens.auth(own_tag)) ) == 1
        trace @"    5. Validity interval has upper bound `ub` and `deadline >= ub + last`"
        expect interval.Interval {
          upper_bound: interval.IntervalBound {
            bound_type: interval.Finite(ub),
            ..
          },
          ..
        } = validity_range
        deadline >= ub + last
      }
      (
        t.Closed(t.ClosedParams { consumer, provider, count: own_count, .. }),
        t.Sub(cont_idx, count, proof),
      ) -> {
        trace @"1. `Closed(shut_params), Sub(cont_idx, count, proof)`"
        expect Some(Output {
          address: cont_addr,
          value: cont_value,
          datum: InlineDatum(cont_idat),
          reference_script: None,
        }) = outputs |> list.at(cont_idx)
        trace @"    1. Cont address is own address"
        expect cont_addr == own_addr
        trace @"    2. `count - open_params.count >= (- diff)`"
        expect [(_, _, cont_ada), (own_hash_, name, _)] =
          cont_value |> value.flatten()
        expect and {
            own_hash_ == own_hash,
            name == own_vali_name,
          }
        let diff = cont_ada - own_ada
        expect count - own_count >= -diff
        trace @"    3. Cont datum is `Done`"
        expect t.Done: t.Dat = cont_idat
        trace @"    4. Provider has signed tx"
        expect extra_signatories |> list.any(fn(s) { s == provider })
        trace @"    5. Verify proof `verify(consumer, count, proof)`"
        verify(consumer, mk_message(own_tag, count), proof)
      }
      (t.Closed(t.ClosedParams { deadline, .. }), t.Drain) -> {
        trace @"1. `Closed(shut_params), Drain`"
        trace @"    1. Validity interval has lower bound `lb` and `deadline <= lb`"
        expect interval.Interval {
          lower_bound: interval.IntervalBound {
            bound_type: interval.Finite(lb),
            ..
          },
          ..
        } = validity_range
        expect deadline <= lb
        trace @"    2. Token twins are burnt"
        let own_mint =
          mint
            |> value.from_minted_value
            |> value.tokens(own_hash)
            |> dict.to_list()
        let vali_tup = Pair(own_vali_name, -1)
        let auth_tup = Pair(tokens.auth(own_tag), -1)
        and {
          own_mint |> list.any(fn(tup) { tup == vali_tup }),
          own_mint |> list.any(fn(tup) { tup == auth_tup }),
        }
      }
      (t.Done, t.Drain) -> {
        trace @"1. `Done, Drain`"
        trace @"    1. Token twins are burnt"
        let own_mint =
          mint
            |> value.from_minted_value
            |> value.tokens(own_hash)
            |> dict.to_list()
        let vali_tup = Pair(own_vali_name, -1)
        let auth_tup = Pair(tokens.auth(own_tag), -1)
        and {
          own_mint |> list.any(fn(tup) { tup == vali_tup }),
          own_mint |> list.any(fn(tup) { tup == auth_tup }),
        }
      }
      _ -> {
        trace @"impossible"
        False
      }
    }
  }
}

fn integer_to_bytearray(endianness: Bool, size: Int, x: Int) -> ByteArray {
  expect and {
      endianness == False,
      size == 0,
    }
  my_integer_to_bytearray(x)
}

/// FIXME : AWAIT PLUTUS V3
pub fn my_integer_to_bytearray(x: Int) -> ByteArray {
  if x == 0 {
    ""
  } else {
    ba.push(my_integer_to_bytearray(x / 256), x % 256)
  }
}

test test_my_integer_to_bytearray() {
  let target = 1232312312
  builtin.integer_to_bytearray(False, 0, target) == my_integer_to_bytearray(
    target,
  )
}
