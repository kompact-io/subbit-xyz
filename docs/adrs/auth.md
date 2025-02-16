---
title: "Auth"
status: proposed
authors: "@waalge"
date: 2025-02-05
tags:
  - auth
---

## Context

Certain actions must be authorised by certain participants. For example: only
the consumer should be able to close and end their account. There are numerous
ways of "doing auth" that have been used in the wild. Some examples:

- By token spend: A particular token (typically an NFT) must appear in the
  inputs or outputs.
- By fixed keys: A verification key hash appears in the `required_signers` field
  of the script context, that matches some hardcoded value, a script param, or
  some value recorded in a datum.
- By signature indicated by a referenced token: a more involved mechanism where
  a utxo in the tx reference inputs is holding some identifiable NFT is held at
  an address, and the corresponding key hash must appear the required signers.

The most appropriate form of auth depends on use case and risk factors. For
example, if key rotation is an important feature, or some more involved on-chain
logic required, then using a token makes this relatively straightforward to
implement. (Send the token to an address corresponding to a new key, or lock the
token at a validator that enforces the more involved auth logic.)

## Decision

### Overview

Use fixed keys for auth over the life of a channel.

There is an option for the consumer to use distinct keys for the hot and cold
parts: one for signing ious, and another for signing txs.

Thus we have:

- An iou key, used for signing ious
- A consumer key, used by the consumer for signing txs
- A provider key, used by the provider for signing txs

The iou key, consumer verification key hash, and provider key hash are available
to the validator logic.

### Rationale

In the context of Subbit.xyz, ious must include signatures and so we require the
use of keys for auth. Mechanics by which the corresponding keys are dynamic over
the lifecycle of the subbit seem non-obvious. Certainly such a mechanism is more
complicated than instead using a fixed verification key to sign all ious. For
this reason the consumer must keep a fixed key for the life of the channel.

The iou signing key is likely available on some "on-line" ("hot") machine. The
use case for Subbit.xyz is where there are many interactions of small value. The
use of an air gapped machine or hardware wallet for signing would be
impractical, and of limited value in this case.

If this machine is compromised the consumer can potentially hand over the
remaining value of the channel to the provider. Note, no-one else can benefit
from the signing key without the providers input. If the key is also used for
transactions, they would potentially be able to close and drain the funds
(immediately by mutual consent).

If instead a different key is used for transaction auth, then the attacker could
at worst create an iou with the remaining channel funds. Only the provider
stands to gain in this case. This is a significant improvement in limiting the
motivations to attempt to get access to the hot keys.

A quirk of Plutus design: it is actually the verification key hash that is
listed in the `required_signers` / `extra_signatories` field, rather than the
verification key. It remains a mystery to me why this was done. It does save us
four bytes and a hash computation if we provide the verification key hash for
the consumer and provider key.

## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

Regarding the use of tokens. An earlier design had opted for tokens as a "tried
and true" method of doing auth. It is more flexible, and the additional overhead
is relatively small. The choice seem moot, and when more evidence comes to light
a version can be created using tokens for tx auth.

Token minting provides a mechanism by which a validator can be invoked on some
"init", and thus check conditions. We are currently of the opinion that, given
only one participants funds are at risk at this point, namely the submitter of
the transaction, it is up to the off-chain code to keep the user safe from
screwing this up. Other participants should also be kept from engaging with
channels which are ill-formed.

## Consequences

Auth by keys leads to a simpler design. As noted we may later have different
auth mechanisms. We design cognisant of this potential extension, without tying
our hands with some overly generic aim.

Since staging steps (ie open) do not require the execution of a validator
on-chain, a provider must comprehensively verify the correctness of a subbit
before engaging with it.
