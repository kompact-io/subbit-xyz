---
title: "iou"
status: proposed
authors: "@waalge"
date: 2025-02-01
tags:
  - iou
---

## Context

The iou is the data object that is passed from the consumer to the provider when
requesting a bit of service. On receipt, the provider verifies its correctness.
In a sub or settle, an iou's correctness is validated on the L1.

We must stipulate the precise definition for of the iou such that its
serialization is consistent where its used.

## Decision

### Overview

```aiken
type Amount = Int
type Signature = ByteArray // 64 Bytes

type Iou {
  amount: Amount
  signature: Signature
}

type IouBody {
  tag : Tag,
  amount : Amount,
}
```

### Rationale

There is not to much to this decision.

The tag is required to allow iou keys to be safely reused. (See
[tags](./tag.md)).

## Discussion, Counter and Comments

.

### Comments

.

### Considered Alternatives

.

## Consequences

.
