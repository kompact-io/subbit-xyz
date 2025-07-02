---
title: "subbit tag"
status: proposed
authors: "@waalge"
date: 2025-02-08
tags:
  - subbit tag
---

## Context

Subbits need to be distinguishable from one another. Moreover ious must
correspond to a single, unique subbit. It would be bad generally bad if an iou
intended for a subbit could be used in another in which it was not intended.

Auth is done by verification keys (see [auth](./auth.md)). We have ruled out
using tokens for auth. A consequence is that we cannot use a token to indicate a
tag. Nor can we use the staging tx (ie open step) to enforce some unique tag
(via the hash of a tx in or otherwise), since no validator is invoked to enforce
this.

Any subbit constants can exist only in the datum (see
[constants](./constants.md)). We have ruled out embedding constants into the
validator via hardcoding or params.

## Decision

### Overview

The subbit tag is fixed by the consumer in the constants.

```ts
type Tag = ByteArray;
```

It is recommended that the consumer use unique tags.

In fact, more explicitly we recommend that the consumer defaults to:

- in the case that a tx opens a single subbit, the tag is the `blake2b256` hash
  of one of the inputs of the open step.
- in the case that a tx opens more than one subbit, the subbit tag is
  `blake2b256` hash of one of the inputs appended with rank of the subbit index.

The body of the iou (the part that is signed) includes the subbit tag, and this
is matched to the derived value in a sub and settle steps.

### Rationale

The context outlines the constraints placed on how a tag can be accommodated,
and excludes a number of ways other projects have managed similar features.

The subbit tag is to keep the consumers ious being re-used in a subbit for which
they were not intended. This would only allow the provider to sub more funds
than they might honestly be owed. No provider funds are ever at risk.

It is the consumer who sets the constants in the open. Thus we leave it to them
to be responsible to set a well chosen tag. Once chosen it cannot be changed in
the usual lifecycle until the subbit is settled.

We leave open the possibility for consumers to set the tag as they see fit. As
long as they do not have two subbits with a common (iou key, tag) pair, then
they are safe from an iou being used in the more than one subbit.

## Discussion, Counter and Comments

This design choice surely will offend some dapp developers and auditors. It
follows the our guiding
[principles of dapp design](https://kompact.io/posts/principles.html)

Allowing duplicate tags allows a potentially important feature (not bug).
Subbits are mono-asset (see [funds](./funds.md)). Opening two subbits with the
same tag, one with currency ada and another with another native asset such as
stable coin, allows currency fluctuations to be softened.

### Comments

Previously this entity was referred to as the "subbit id". However, "id"
conveyed that this _ought_ to be globally unique amongst subbits. Since it is
not, "tag" is considered more accurate.

### Considered Alternatives

.

## Consequences

We rely on a consumer's self interest that they won't unintentionally reuse a
pair (iou key, tag) any more than they would share a signing key.

It is relatively straightforward to scan the chain to see all such pairs used. A
consumer in doubt should stop using the iou key.

Tag reuse is not a problem for a provider. The provider is responsible for
ensuring that there exists at least one subbit with acceptable state when
accepting an iou. They should be able to handle the case that there is more than
one.

The provider should not handle subbits that they deem ill formed. This includes
the case that a tag is "excessively long". We do not place a hard limit on what
counts as excessive. By asking customers to have short tags, the provider may be
able to save a little in tx fees, with no impact on safety.
