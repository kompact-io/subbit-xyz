---
title: "subbit id"
status: proposed
authors: "@waalge"
date: 2025-02-08
tags:
  - subbit id
---

## Context

Subbits need to be distinguishable from one another. Moreover ious must
correspond to a single, unique subbit. It would be bad generally bad if an iou
intended for a subbit could be used in another in which it was not intended.

Auth is done by verification keys (see [auth](./auth.md)). We have ruled out
using tokens for auth. A consequence is that we cannot use a token to indicate
an id. Nor can we use the staging tx (ie open step) to enforce some unique id
(via the hash of a tx in or otherwise), since no validator is invoked to enforce
this.

Any subbit constants can exist only in the datum (see
[constants](./constants.md)). We have ruled out embedding constants into the
validator via hardcoding or params.

## Decision

### Overview

The subbit id is fixed by the consumer in the constants.

```
type SubbitId = ByteArray
```

It is highly recommended that the consumer use unique ids. In fact, more
explicitly we recommend that:

- in the case that a tx opens a single subbit, the subbit id is the `blake2b256`
  hash of one of the inputs of the open step.
- in the case that a tx opens more than one subbit, the subbit id is
  `blake2b256` hash of one of the inputs appended with rank of the subbit index.

The body of the iou (the part that is signed) includes the subbit id, and this
is matched to the derived value in a sub and settle steps.

### Rationale

The context outlines the constraints placed on how a subbit id can be
accommodated, and excludes a number of ways we might have naturally hoped to do
this.

The subbit id is to keep the consumers ious being re-used in a subbit for which
they were not intended. This would only allow the provider to sub more funds
than they might honestly be owed. No provider funds are ever at risk.

It is the consumer who sets the constants in the open. Thus we leave it to them
to be responsible to set a well formed subbit id.

We do leave open the possibility for consumers to set alternative subbit ids
that may align better with the application.

Importantly having duplicate subbit ids is not impossible, and should be handled
sensibly by all tooling. Allowing duplicate ids allows a potentially important
feature (not bug). Subbits are mono-asset (see [funds](./funds.md)). Opening two
subbits with the same id, one with currency ada and another with another native
asset such as stable coin, allows currency fluctuations to be softened.

## Discussion, Counter and Comments

This design choice surely will offend some dapp developers and auditors. It
follows the our guiding
[principles of dapp design](https://kompact.io/posts/principles.html)

### Comments

.

### Considered Alternatives

.

## Consequences

We rely on users keeping themselves safe by not reusing the same constants in
two separate subbits. Provided that they track all previous subbits associated
with a key, this is easily done.
