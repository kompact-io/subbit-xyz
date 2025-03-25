---
title: "L1 Test & Bench"
authors: "@waalge"
date: 2025-02-15
---

## Intro

### Aims

To test and bench the Subbit.xyz validator.

### Output for M1

According to our milestone
[M1](https://milestones.projectcatalyst.io/projects/1300174/milestones), it must
be demonstrated that:

1. The SC aligns with spec. SC builds. Tests succeed. Benchmarks run.

2. The output of benchmarks are numbers providing evidence of the feasibility
   and costing of "typical txs"

3. There are at least 10 tests, with some covering positive conditions, some
   negative.

There is some acknowledgement that this is a proxy, albeit a poor one, for a
"sensible" amount of testing.

### Preample on Cardano Txs

Cardano txs have fees that increase with size and complexity. Moreover, there
are upper bounds on these attributes that cannot be exceeded.

See
[this guide](https://docs.cardano.org/about-cardano/explore-more/parameter-guide)
for a description and further signposting on parameters, their meanings, and
purpose.

The more inputs or outputs a tx has, the larger size it will have. Each
validator that is invoked, adds to the complexity budget of the tx, and may also
contribute the tx size or additional reference input budget.

#### Bounds

- `maxTxSize` - Max total bytes of tx. This excludes the size of inputs, beyond
  their output reference and likewise for reference inputs.
- `maxTxExecutionUnits.exUnitsMem` - Max mem units that can be accumulated in a
  single tx.
- `maxTxExecutionUnits.exUnitsSteps` - Max cpu units that can be accumulated in
  a single tx.

At the time of writing:

- `maxTxSize = 16384`
- `maxTxExecutionUnits.memory = 14000000`
- `maxTxExecutionUnits.steps = 10000000000`

#### Fees

The relevant params with there current values are:

- `txFeePerByte` aka `minFeeA` ` := 44`
- `txFeeFixed` aka `minFeeB` ` := 155381`
- `executionUnitPrices` `:= { priceMemory: 5.77e-2, priceSteps: 7.21e-5 }`
- `minFeeRefScriptCostPerByte := 15`

Each time a script is executed as part of the transaction validation process, it
adds to the memory and cpu budget.

Let `size` be the tx size (in bytes), and `mem` and `cpu` be the total units of
memory and cpu usage. Let `scriptSize` be the total bytes of ref scripts in
either the inputs or reference inputs. Then the fee computation is:

```math
fees
  = txFeeFixed
  + txFeePerByte * size
  + mem * priceMemory
  + cpu * priceSteps
  + scriptSize * minFeeRefScriptCostPerByte
```

There are two simplification in the above:

- By first totalling the `mem` and `cpu` values we introduce a potential
  rounding error. The above may give a very small underestimate. We should
  instead find the `ceil` for the potentially non integer value cost
  corresponding to each redeemer.
- The reference script cost calculation is generally non-linear, but for our
  purposes only the linear part is our concern. A note on the matter is
  [here](https://github.com/IntersectMBO/cardano-ledger/blob/e5924afa4795ea9503165adaf4fc902c005d69b3/docs/adr/2024-08-14_009-refscripts-fee-change.md#formula-for-the-cost-due-to-reference-script-usage)

### Aiken Benchmarks

Aiken has an inbuilt benchmarking tool
[bench](https://aiken-lang.org/language-tour/bench). To use it we need to define
appropriate fuzzers for the function under consideration. It will return the cpu
and mem usage.

## SC/ Spec alignment

The spec is structured in a way that roughly mirrors that of the implementation.
This likely makes the spec harder to read and digest than some alternative
presentation. However, once this obstacle has been overcome, it should be much
easier to assure oneself that the code does all the things the spec indicates it
should.

In fact key parts of the spec are inlined in the code. Small divergence will
creep in and will be fix on an ongoing basis, particularly in
[M6](https://milestones.projectcatalyst.io/projects/1300174/milestones/6). Large
divergences should be fixed immediately. (We hope for literate aiken files
someday).

## Tests

### Steps succeed

We fuzz each step individually:

```sample
$aiken check -m "step.{..}"
    Compiling kompact-io/subbit-xyz 0.0.0 (.)
    Resolving kompact-io/subbit-xyz
      Fetched 1 package in 0.05s from cache
    Compiling aiken-lang/stdlib v2.2.0 (./build/packages/aiken-lang-stdlib)
    Compiling aiken-lang/fuzz main (./build/packages/aiken-lang-fuzz)
   Collecting all tests scenarios within module(s): *step*
      Testing ...

    ┍━ mark/steps ━━━━━━━━━━━━━━━━━━━
    │ PASS [after 100 tests] test_add
    │ PASS [after 100 tests] test_sub
    │ PASS [after 100 tests] test_close
    │ PASS [after 100 tests] test_settle
    │ PASS [after 100 tests] test_end
    │ PASS [after 100 tests] test_expire
    ┕ with --seed=2867413678 → 6 tests | 6 passed | 0 failed
```

The fuzzers for these aren't the most elaborate, but we deem them good enough.

### Steps fail

We do some sanity checks that things that ought to fail do. For every step we
have a `{{step}}_not_signed` to check the expected party has signed. In addition
we have the following:

1. `add_less` : An add step in which the continuing output has less funds.
2. `sub_too_much` : A sub that in which the continuing output has less funds
   than should be allowed by the iou
3. `sub_bad_id` : A sub in which the iou is for a different subbit
4. `sub_bad_sig` : A sub in which the iou has a bad id
5. `close_bad_data` : A close in which the continuing output datum has changed,
   other than the stage
6. `close_bad_expire` : A close in which the expire at timestamp is too soon
7. `expire_too_soon` : An expire in which the time lower bound is not after
   expire at.

### Mutual

TODO

## Benchmarks

We want costings estimates of "typical" txs. Namely, txs in which steps are done
in batch. For Subbit.xyz, each subbit spend is one invocation of the script.

### Batched sub

The batch sub is the typical transaction of a provider. They take their lastest
set of ious of a batch of subbits, and submit all of these to the L1. This is
the most relevant type of tx to consider.

The following is the costing of our slightly crude "batch subbit simulator
function". The function invokes all but some preamble of the subbit validator
for each subbit input.

```sample
┍━ mark/steps ━━━
│ test_multi_subs
│   memory units                                           cpu units
│   ⡁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⢠⠓⡁ 14161961.0   ⡁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⠀⠁⠈⡠⠓⡁ 7878806528.0
│   ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠤⠊⠁⠀⠄              ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠔⠊⠀⠀⠄
│   ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⠔⠁⠀⠀⠀⠀⠂              ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠤⠊⠀⠀⠀⠀⠀⠂
│   ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠤⠃⠀⠀⠀⠀⠀⠀⠀⡁              ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠤⠒⠁⠀⠀⠀⠀⠀⠀⠀⡁
│   ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡔⠚⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄              ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠤⠊⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄
│   ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⡠⠒⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂              ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠔⠊⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂
│   ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠤⠊⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡁              ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠒⠊⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡁
│   ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠤⠊⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄              ⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠤⠒⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄
│   ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⡠⠔⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂              ⠂⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠔⠊⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂
│   ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠤⠒⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡁              ⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠔⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡁
│   ⠄⠀⠀⠀⠀⠀⠀⢀⠤⠒⠊⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄              ⠄⠀⠀⠀⠀⠀⢀⡠⠒⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠄
│   ⠂⠀⠀⢀⠤⠒⠊⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂              ⠂⠀⢀⣀⠤⠊⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠂
│   ⠥⠲⠊⠅⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠁ 238348.0     ⠥⠪⠁⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠠⠀⠄⠁ 142985712.0
│   1.0                                 50.0               1.0                                 50.0
┕━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ with --seed=1817129587
```

The complexity/ size measure in the x-axis corresponds to the number of subbits
involved. Both memory and steps (aka cpu) grow essentially linearly.

In conclusion: with 50 sub steps just about hits the max memory limit
(14161961 > 14000000). See below if we expect to reach this many inputs before
hitting the max tx size.

### Counting Bytes:

#### Validator

The following functions outputs the size in bytes of the subbit validator:

```bash
echo $(( ($(cat plutus.json | jq '.validators[0] | .compiledCode' | wc -m) - 3) / 2))
```

The following numbers are given with the current code base and versions of aiken
etc.

Aiken provides the ability to build the script with different levels of tracing.
With:

- no tracing - `3662` bytes.
- compact, user-defined tracing - `4491` bytes.
- full verbose tracing - `10057`

Without tracing, the validator is on-par with a simple to moderate complexity
validator found in the wild.

Even with full tracing, the script can be output as a reference script without
offending the `maxTxSize` limit. A tx including the validator as a reference
script will pay `54930` lovelace more in fees than the identical tx without the
reference script.

#### Tx

The three or four fields that grow linearly in the number of subbits input. All
other fields are constant. The three fields that grow linear are the inputs,
outputs, and redeemers. The fourth is the "required signers" (cardano ledger/
plutus) or "extra signatories" (aiken) field - and it depends on whether or not
the provider reuses their key. Lets assume here there is some key reuse.

Our numbers are very rough, back of an envelope level calculations.

Size of an input is the size of an output reference `~= 36`.

Size of redeemer. The redeemer includes an Iou, consisting of an amount and
signature.

```
  amount ~= 8
  sig ~= 64
  TOTAL + WRAPPER ~= 74
```

A redeemer includes a constructor wrapped output reference `~= 36 + 2 = 38` The
pair is then `38 + 74 + 2= 114`

Size of the constants

```
  subbit_id ~= 32
  currency ~= 40
  iou_key ~= 32
  consumer ~= 28
  provider ~=28
  close_period ~= 8
  TOTAL + WRAPPER ~= 172
```

An opened datum is then `~= 180`

Size of the output

```
  address ~= 60
  value ~= 20
  datum ~= 182
  TOTAL = 262
```

Adding all this up, we estimate that including a subbit in the tx adds `~= 592`
bytes. The max tx size is `16348`, and there are a couple 100 bytes of "general
overhead". Thus we hope to be able to handle `> 22` subs per tx, and perhaps
with more restricted cases (eg Ada only, no staking) we might see `~ 30` subbits
per tx.

A final note: the datum is a substantial part of the byte budget. If the datum
was embedded then we could see the bytes per subbit drop to `~= 420`. Perhaps
with further optimizations we might be able to get close to double our `22`
figure.
