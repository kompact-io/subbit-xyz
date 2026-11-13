//// Benchmarks modelling the "typical claim" flow: a single `Main`-redeemer
//// transaction settling `N` simultaneous `Sub` steps against `N` distinct
//// channels at once (e.g. a relayer batching many claims together).
////
//// Each of the `N` channels draws from a small, fixed pool of distinct
//// consumer/provider/IOU-key fixtures (see `subbit/fixtures.claim_at`), so
//// `extra_signatories` and the ordered vkh list genuinely grow with `N` up
//// to the pool size, rather than collapsing to a single deduplicated
//// entry. Cardano's tx-size limit caps how large `N` can realistically be
//// in a single transaction anyway, so these numbers are informative more
//// than load-bearing.

use aiken/collection/list
use aiken/fuzz
use cardano/assets
use cardano/transaction.{Transaction, placeholder}
use subbit/fixtures as fx
use subbit/spend/main.{do}
use subbit/types as t

fn sub_step(claim: fx.BenchClaim) -> t.Step {
  t.StepCont(t.Sub { n: claim.owed, sig: claim.sig })
}

fn scenario(size: Int) -> (List<t.Step>, Transaction) {
  let indices = list.range(0, size - 1)
  let steps = indices
    |> list.map(fn(i) { sub_step(fx.claim_at(i)) })
  let inputs = indices
    |> list.map(
        fn(i) {
          let claim = fx.claim_at(i)
          fx.input_with(
            fx.oref(i),
            claim.constants,
            t.Opened { subbed: 0 },
            assets.from_lovelace(2000),
          )
        },
      )
  let outputs = indices
    |> list.map(
        fn(i) {
          let claim = fx.claim_at(i)
          fx.output_with(
            claim.constants,
            t.Opened { subbed: claim.owed },
            assets.from_lovelace(1000),
          )
        },
      )
  let tx = Transaction {
    ..placeholder,
    inputs,
    outputs,
    extra_signatories: fx.bench_providers,
  }
  (steps, tx)
}

fn sampler(size: Int) -> Fuzzer<Int> {
  fuzz.constant(size)
}

bench sub_claims(size via sampler) {
  let (steps, tx) = scenario(size)
  do(fx.datum(t.Opened { subbed: 0 }), steps, tx)
}

test sub_claims_scenario_is_actually_valid() {
  let (steps, tx) = scenario(10)
  do(fx.datum(t.Opened { subbed: 0 }), steps, tx)
}

test sub_claims_scenario_handles_zero() {
  let (steps, tx) = scenario(0)
  do(fx.datum(t.Opened { subbed: 0 }), steps, tx)
}
