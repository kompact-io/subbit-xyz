---
title: "funds"
status: proposed
authors: "@waalge"
date: 2025-02-08
tags:
  - funds
---

## Context

Cardano since the Mary era supports "native assets". Native assets behave in the
ledger very similarly to the core currency, ada. For example, at any point in
time a native asset at tip necessarily appears in the value field of a single
utxo.

Native assets can be minted and burned according to custom logic, sometimes
called the minting script, minting policy, currency policy _etc_. Our preferred
term is simply "validator" which is "invoked with purpose mint". We might use
the term "token", "asset", "asset class" almost interchangeably.

This ADR is to provide a decision on whether a subbit funds are:

- only ada
- one native asset
- multi-assets

## Decision

### Overview

A subbit supports mono-asset funds. The asset class is the **currency** of the
subbit. There is explicit handling of: only ada, by script hash, and by asset
class.

```aiken
type Currency {
  Ada,
  ByHash(ScriptHash)
  ByClass(ScriptHash, AssetName)
}
```

### Rationale

The decision to not be ada only is based on a hunch. The hunch being that
providers would be keen to charge for services in a way that most accurately
reflects their costs. Many services involve cloud infrastructure, which is
defacto billed for in fiat currency such as usd. Thus there will be interest in
the ability to use "stable coins" or similar native assets.

Many stable coins are the only assets of the script hash, thus we need not
handle the asset name.

The option of multi-asset subbits might be of interest in some application.
However, it surely introduces additional complexity. We await requests for such
functionality in order to begin to interrogate its utility and design
implications further.

## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

.

## Consequences

The subbit datum must record its own currency. See the ADR on
[constants](./constants.md).

Ious will include an amount, and this is a single integer, rather than a more
complex data structure such as a list or map.
