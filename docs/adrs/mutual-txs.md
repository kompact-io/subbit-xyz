---
title: "Mutual txs"
status: proposed
authors: "@waalge"
date: 2025-02-05
tags:
  - mutual txs
---

## Context

A key aim of distributed ledger tech is to keep users safe from malicious
actors: first by distributing power to say what the state actually is (ie
"distributed ledger"), and by using cryptography to make certain statements
irrefutable (eg "I consented to this transaction with my signature) or at least
very expensive to lie in particular and consequential ways.

At any point a subbit's funds belong to the consumer, minus the amount owed to
the provider and attested for in the latest iou. In the standard subbit
lifecycle, each step is submitted by one of the subbit participants. The
validator protects each participant from malicious actions of the other, and
users of the wider network. These txs are necessarily restrictive.

The question is whether to allow transactions that are mutually agreed upon.
Since the subbit funds belong to the two participants, then if they both consent
to a tx, is this enough to be deemed permissible.

## Decision

### Overview

Allow mutual txs that can perform arbitrary update of state, including
terminating the subbit.

A mutual tx must be spent with the redeemer `Mutual`. Both participants must
have signed the tx. No other subbit can be present in the tx.

### Rationale

Supporting this feature comes down a judgement call on whether the
implementation overhead and additional potential risk are outweighed by
benefits.

The overhead of supporting mutual txs is not none, but is small. It consists of
handling an additional pathway that checks "have both participants signed". By
specifying no other subbit can be spent in the same tx excludes having to
re-consider the arguments presented in [batching](./batching.md).

Strictly speaking there is no additional risk to a participants funds. A
consumer could just as easily send an iou for the total of their subbit funds
(an action necessarily supported), as they could sign a transaction closing the
subbit and subsequently handing all their funds to the provider.

The ability to perform "arbitrary" txs spending a subbit does introduce risks.
The off-chain code must communicate transparently to each participants what it
is the tx they are consenting to _does_. This is no more the case than is
generally so when transacting with any dapp.

The action is suitable when subbit participants are cooperative, and responsive.
It has the potential to do things not possible in the standard lifecycle. For
example change signing keys, or terminating a subbit and opening a new one in a
single tx.

The benefits are potentially muted, by participating choosing to instead batch
their own actions. For example a provider performing many subs in a single
transaction. (See [batching](./batching.md).)

## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

An alternative might be to either:

- opt out of this feature
- have additional logic limiting the scope of such actions.

## Consequences

An additional pathway in the L1.

L2 tooling might consider how to ergonomically expose this, but we postpone
developing this to another time.
