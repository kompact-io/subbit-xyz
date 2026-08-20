use aiken/collection/list
use aiken/crypto.{VerificationKey, VerificationKeyHash}
use aiken/interval.{Interval, IntervalBound}
use cardano/address.{Address, Credential, Script}
use cardano/transaction.{Input, Output, Transaction}
use extra/xlist.{foldl3}
use subbit/io.{yield_in, yield_out}
use subbit/prelude.{Timebound}
use subbit/steps/add
use subbit/steps/close
use subbit/steps/elapse
use subbit/steps/end
use subbit/steps/settle
use subbit/steps/sub
use subbit/types as t

/// In a standard tx, the "main" invocation does all the verification steps for all
/// channels.
pub fn do(datum: t.Datum, steps: List<t.Step>, tx: Transaction) {
  let Transaction { inputs, outputs, validity_range, extra_signatories, .. } = tx
  let  
      Interval {
        lower_bound: IntervalBound { bound_type: lower_bound, .. },
        upper_bound: IntervalBound { bound_type: upper_bound, .. },
      }
    = validity_range
  let (own_hash, _, _) = datum
  let own_cred = Script(own_hash)
  let with = mk_with(own_cred, lower_bound, upper_bound, _, _, _, _, _)
  let inputs, _outputs, vkhs <- foldl3(steps, inputs, outputs, [], with, _)
  expect
    vkhs
      |> list.all(fn(vkey) { extra_signatories |> list.has(vkey) })
  inputs
    |> list.all(fn(i) { i.output.address.payment_credential != own_cred })
}

type WithReturn<result> =
  fn(List<Input>, List<Output>, List<VerificationKey>) -> result

fn mk_with(
  own_cred: Credential,
  lb: Timebound,
  ub: Timebound,
  step: t.Step,
  inputs: List<Input>,
  outputs: List<Output>,
  vkhs: List<VerificationKeyHash>,
  return: WithReturn<result>,
) -> result {
  let delegation, v_in, c, s_in, inputs <- yield_in(own_cred, inputs)
  when step is {
    t.StepCont(step) -> {
      let v_out, s_out, outputs <- yield_out(own_cred, delegation, c, outputs)
      let vkey = when step is {
        t.Add -> add.do(c, v_in, s_in, v_out, s_out)
        t.Sub { n, sig } -> sub.do(c, v_in, s_in, v_out, s_out, n, sig)
        t.Close -> close.do(ub, c, v_in, s_in, v_out, s_out)
        t.Settle { n, sig } -> settle.do(c, v_in, s_in, v_out, s_out, n, sig)
      }
      let vkhs = vkhs |> xlist.ordered_insert(vkey)
      return(inputs, outputs, vkhs)
    }
    t.StepEol(step) -> {
      let vkey = when step is {
        t.End -> end.do(c, s_in)
        t.Elapse -> elapse.do(lb, c, s_in)
      }
      let vkhs = vkhs |> xlist.ordered_insert(vkey)
      return(inputs, outputs, vkhs)
    }
  }
}
