---
title: "Constants"
status: proposed
authors: "@waalge"
date: 2025-02-05
tags:
  - params
---

## Context

Each subbit has values that are particular to the subbit, such as the
credentials of the participants and the currency.

These values endure throughout the life of the subbit. 
Thus these could be hard coded into the validator, or fixed in the
params, or set in the datum and verified on each transaction.

## Decision

### Overview

All const values are held in the subbit datum.

The constants are

```aiken
type Constants {
  subbit_id : SubbitId,
  currency : Currency,
  iou : VerificationKey,
  consumer : VerificationKeyHash,
  provider : VerificationKeyHash,
  close_period : Int,
}
```
- See [subbit-id](./subbit-id.md) for the definition of `SubbitId`.
- See [auth](./auth.md) for the relevance of the verification keys.
- See [lifecycle](./lifecycle.md) for the meaning of the close period
- See [funds](./funds.md) for the definition of `Currency`. 

The opened stage is fixed point of steps `add`, and `sub`, and the constants must indeed remain constant.
The closed stage requires all but the close period. However we leave it included, to reduce the number of checks in the close step.
The settled stage requires only the consumer.

### Rationale

The decision makes batching straightforward. 
Embedding values into the validator, either by hardcoding them or using params,
(which is effectively hardcoding, badly organized) results in a different
validator, and thus different validator hash.
Subbits are held a single address, regardless of the participants or currency. 
See [batching](./batching.md). 

It also makes marketing easier. All subbits exist at a
single "subbit address". It is easy to track and easy to make claims of usage
and "TVL".

However, the additional flexibility does have additional overheads. Every step
must check the constants are preserved.

In typical use, a subbit spends almost all its life at the opened stage. 


## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

It would be more efficient, and simplify the code base to hardcode the values.
This does require the validator to be recompiled before use, which has an
additional overhead but in the much less resource constrained context of
off-chain.

Using params is more cumbersome than hardcoding since the values must be passed
down from the top of the function down to each place the value is used.
Otherwise, it shares the benefit of not requiring the preservation of constants
check.

If a version with either of these is of interest to future users, we can revisit
this decision.

## Consequences

There will be more checks on each action on the subbit than would be the case
otherwise.
