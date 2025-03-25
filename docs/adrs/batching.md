---
title: "Batching"
status: proposed
authors: "@waalge"
date: 2025-02-05
tags:
  - batching
---

## Context

Providers are likely serving many consumers. Consumers likely have multiple open
subbits.

Each channel action costs fees. Subbit.xyz should be as cheap as possible
without compromising safety, and convenience.

## Decision

### Overview

Subbit steps can be batched into single txs. Moreover this is treated as first
class.

On any tx, the lexicographical first subbit input must have a redeemer
`Main(steps)`. All other subbit redeemers are simply `Batch`. The validator
logic finds the first validator input. Note this logic is executed for each
input.

The first validator input is the "main" input. If "own is main" (ie main output
reference is `own_oref`) then the redeemer must be `Main(steps)` else must be
`Batch`.

If is main, then the validator continues with all remaining logic. Else, the
validator returns true.

The datum and the redeemer have the following forms

```aiken
type Datum {
  own_hash : ByteArray
  ...
}

type Redeemer {
  Batch
  Main(List<Step>)
  ...
}
```

The order of:

1. validator inputs,
2. `steps` in `Main(steps)`
3. validator outputs (where expected)

All agree.

### Rationale

We anticipate batching, txs involving 2 or more subbits, to be standard form of
txs for providers. It is much cheaper and quicker for providers to sub in batch
than one tx at a time. Depending on tooling, perhaps this will also be the case
for consumers.

One awkwardness of Cardano is that each utxo spent from a validator address
invokes the validator, and so adds to the tx cost. An approach to accommodate
this, is to de-duplicate logic executed by delegating to a single invocation
associated to some designated "main" input.

Another awkwardness of Cardano spends is establishing "own hash" in order to
identify other inputs from the same validator address, and for example identify
the main validator input.

Own hash can be found by folding over transaction inputs, matching on
`own_oref`, and extracting script hash from the address For example:

```aiken
fn get_own_hash(own_oref : OutputReference, inputs : List<Input>) : ScriptHash {
  expect [input, ..inputs] = inputs
  if own_oref == input.output_reference {
    expect ScriptCredential(own_hash) = input.output.address.payment_credential
    own_hash
  } else {
    get_own_hash(own_oref, inputs)
  }
}
```

This can be made more efficient if we can inform the validator the index of the
correct input. In doing, fewer deconstructions and comparisons are required.
However, the cost remains non-negligible and grows quadratically on script
inputs.

An alternative approach is to "cache" own hash in the datum. This is then
immediately available to the validator logic. This makes finding the main
validator input linear. Setting the "main" input to be the lexicographically
first validator input in the inputs is the cheapest possible choice.

Note that the design choice made in [auth](./auth.md) opted for not using
tokens. Thus we cannot depend on tokens to provide witness for datum
correctness. Even if we were using tokens, this still requires finding own input
in the inputs in order to interrogate own value (depending on what needs to be
verified).

We will come back to "what if bad datums".

The logic for each invocation of the script is as follows:

1. Unpack `own_hash` and derive `own_cred = ScriptCredential(own_hash)`.
2. Find first input, `main`, from inputs by address payment credential matching
   `own_cred`.
3. Expect `main` datum to have form above, and matching `own_hash`.
4. If `main` output reference is `own_oref` then continue, else true.

The main input then continues with all remaining logic. The redeemer of main
includes the steps for all inputs. This will include insuring all relevant
outputs have correct `own_hash` in datum.

The ordering constraint aids the implementation. Step logic is completely
decoupled from one subbit to another: there is no need to, say, aggregate values
across subbits. The logic can recurse over inputs, steps, and outputs. Each
input expects a corresponding step. In the case that a step is non-terminal
there is a corresponding output. In the case that a step is terminal there is no
corresponding output. The logic must exhaust all inputs and steps.

We return to considering _What if the datum is bad?_. Specifically, what if
there is a utxo at the validator address with `datum.own_hash` value is
incorrect.

According to our [guiding principles](https://kompact.io/posts/principles.html),
we design validators (almost) exclusively to keep users safe from one another.
Validators are not concerned keeping users safe from themselves. Tooling should
always make it hard for users to shoot themselves in the foot, but if they
really want to, we let them. Sometimes its not possible for code to distinguish
between a user shooting themselves in the foot, or doing something quite
sensible.

The value of an offending utxo must belong to the author of the transaction.
Such a utxo cannot have been a continuing output. This follows from the
validator logic ensuring continuing outputs have `own_hash` set correctly. So as
far as we can discern, the author of the tx outputting a bad utxo is risking
their own value and only their own value.

If an offending utxo, `bad_input`, is spent with redeemer `Main(steps)`, then
the tx will fail as either:

- it will not find `main` (since `own_cred` will not match), or
- `main` will not be itself (wont match on `own_oref`)

If the offending utxo, is spent with `Batch`, then it will not pick up the
correct `main` input. The utxo may or may not be spendable, depending on whether
the validator finds some `incorrect_main` and this is spendable.

Including a `bad_input` in a tx with good inputs will either fail, or be of no
consequence. A good input only delegates logic to `main_input`, so things only
go wrong if `main_input == bad_input`. But in this case the good input will fail
when expecting a match between `own_hash` and that of the bad input.

## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

.

## Consequences

This has implications on validator logic design.
