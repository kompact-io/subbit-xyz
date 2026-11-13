---
title: "x402 batch-settlement binding: Cardano (Subbit)"
status: draft
authors: "@waalge"
date: 2026-11-06
---

## Summary

This binds x402's `batch-settlement` scheme to Cardano using Subbit, an Aiken payment-channel
validator. A consumer deposits funds into an on-chain channel once; each request produces an
off-chain, Ed25519-signed, cumulative IOU; the provider redeems accumulated IOUs on its own
schedule, batching many channels into one transaction. Trust is capital-backed: the consumer's
own deposit, not a third party, backs every IOU.

This binding defines **self-managed mode only**. §9 explains why no facilitator-managed mode is
defined here, and it is a structural claim about this validator, not a policy preference.

This document uses Subbit's own vocabulary throughout. §1 is a glossary against x402's own terms
for readers coming from other bindings.

## 1. Glossary

| Subbit term    | Closest x402 term                | Note                                                                      |
| -------------- | -------------------------------- | ------------------------------------------------------------------------- |
| subbit         | channel                          | the on-chain escrow UTxO plus its datum                                   |
| open           | deposit                          | first funding of a channel                                                |
| add            | deposit (top-up)                 |                                                                           |
| tag            | — (see §4.3)                     | **not** a channel identifier in the x402 sense; see divergence below      |
| (iou_key, tag) | channelId                        | the pair is the practical commitment identifier; neither half alone is    |
| IOU            | voucher                          | Ed25519 signature over `(tag, amount)`; amount is cumulative              |
| sub            | claim                            | mid-life redemption; channel stays open                                   |
| close          | — (unilateral exit, step 1 of 2) | consumer-initiated; starts the exit window                                |
| settle         | claim (final)                    | provider's last redemption, after close, before elapse                    |
| elapse         | timed withdrawal (payer side)    | consumer reclaims after the exit window, if provider never settled        |
| end            | —                                | consumer's cleanup step after settle; x402 has no equivalent              |
| mutual         | cooperative refund               | both parties co-sign; validator enforces consent only, not the split (§8) |
| consumer       | payer / client                   |                                                                           |
| provider       | receiver / server                | **not** a preset address — see §4.2, §9                                   |
| Keytag         | commitment identifier            | `iou_key ‖ tag`; not guaranteed unique (§4.3)                             |

## 2. Network identifiers

`cardano:mainnet`, `cardano:preprod`, `cardano:preview` (CAIP-2, human-readable form). CIP-34's
numeric form (`cip34:1-764824073`, `cip34:0-1`, `cip34:0-2`) MUST be accepted as an input alias
and normalized before matching, consistent with this scheme family's existing Cardano `exact`
binding.

## 3. Trust model

**Capital-backed.** The channel is a UTxO at the Subbit validator's script address, funded
entirely by the consumer. No intermediary underwrites the provider's eventual payout; the
provider's guarantee of eventual settlement is exactly the guarantee the Cardano ledger gives any
UTxO it can produce a valid redeemer for.

## 4. Channel model

### 4.1 Datum

```
Datum     = (own_hash: ScriptHash, Constants, Stage)
Constants = (tag: Bytes, asset: Asset, iou_key: Ed25519VerificationKey,
             consumer: VerificationKeyHash, provider: VerificationKeyHash, close_period: Int)
Asset     = Lovelace | Native(policy: ScriptHash, name: Bytes)
Stage     = Opened(subbed: Int) | Closed(subbed: Int, elapse_at: Int) | Settled
```

`Constants` never changes for the life of a channel; every step either preserves it exactly or
terminates the channel. `close_period` is milliseconds; `elapse_at` is POSIX milliseconds, fixed
at `Close` time to (at least) the closing transaction's own validity upper bound plus
`close_period`.

### 4.2 Redeemers and steps

```
Redeemer = Defer | Main(List<Step>) | Mutual
Step     = Add | Sub(n: Int, sig: Signature) | Close
         | Settle(n: Int, sig: Signature) | End | Elapse
```

| Step             | Stage in → out                                 | Required signer                | Effect                                                                                                                |
| ---------------- | ---------------------------------------------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `Add`            | `Opened` → `Opened` (unchanged)                | consumer                       | value in channel strictly increases                                                                                   |
| `Sub(n, sig)`    | `Opened` → `Opened`                            | **provider**                   | IOU `(tag, n)` verifies under `iou_key`; `n ≥ subbed_out`; value leaves the channel equal to `subbed_out − subbed_in` |
| `Close`          | `Opened(subbed)` → `Closed(subbed, elapse_at)` | consumer                       | `subbed` unchanged, value unchanged, validity upper bound ≤ `elapse_at − close_period`                                |
| `Settle(n, sig)` | `Closed` → `Settled`                           | **provider**                   | IOU verifies; provider takes at most `n − subbed_in`                                                                  |
| `End`            | `Settled` → (spent)                            | consumer                       | terminal cleanup; no further continuing output                                                                        |
| `Elapse`         | `Closed` → (spent)                             | consumer                       | only once the tx's validity lower bound ≥ `elapse_at`; the consumer's safety valve if the provider never settles      |
| `Mutual`         | any → (spent)                                  | **both** consumer and provider | no other check — see §8                                                                                               |

Claiming funds (`Sub`, `Settle`) and _authorizing_ that claim are the same act: the provider's own
signature on the spending transaction. There is no separate "verify this claim" step a third
party could perform instead.

**Batching.** One transaction may redeem many channels at once. Exactly one channel input per
transaction carries `Main` with a step for every channel input present (including its own);
every other channel input in that transaction MUST carry `Defer`. The validator enforces this by
requiring the lexicographically-first channel input to be the one carrying `Main`.

### 4.3 The IOU, and why its identifier isn't a channel identifier

An IOU is an Ed25519 signature by `iou_key` over the Plutus Data pair `(tag, amount)` — the same
cumulative-ceiling mechanic as an EVM voucher's `maxClaimableAmount`.

`tag` is chosen by the consumer when opening the channel and is **not required to be unique**.
This is a deliberate design choice (see `docs/adrs/tag.md`), not an oversight: nothing in the
validator can enforce uniqueness, since no script runs on `open` itself, and tag reuse enables a
genuine feature — the same `(iou_key, tag)` pair spanning channels of different asset types lets
a consumer soften currency fluctuations across them. The convention this binding recommends
(derive `tag` from an input the opening transaction spends) reduces _accidental_ collision but is
not, and cannot be, enforced on-chain.

The practical consequence: `Keytag = iou_key ‖ tag`, used as this binding's commitment
identifier, satisfies the parent scheme's "non-empty identifier on success" requirement, but
**does not uniquely pick out one channel**. A provider MAY see more than one `Opened` channel
under the same `Keytag` and MUST be prepared to pick among them (e.g. preferring the one with
outstanding uncommitted value, tie-broken by lowest `subbed`). Tooling built against this binding
MUST NOT assume commitment-identifier uniqueness the way an EVM `channelId` or similar
deterministic identifier would provide it.

## 5. Wire format

Headers, not a JSON body: `PAYMENT-SIGNATURE` (client → server), `PAYMENT-REQUIRED` /
`PAYMENT-RESPONSE` (server → client), each base64-encoded JSON.

### 5.1 `PaymentRequirements.extra` (provider terms)

| Field                | Type                 | Meaning                                                                                                                                                             |
| -------------------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `payment_flow`       | `"upfront"`          | commit happens before the resource is served, with no second settle call after; MUST be declared explicitly rather than left to the generic `authorization` default |
| `script`             | 28-byte hash         | the validator's script hash; the client MUST accept this before engaging                                                                                            |
| `asset`              | `Asset`              | required currency, as in `Constants.asset`                                                                                                                          |
| `provider`           | 28-byte hash         | required provider key, as in `Constants.provider`                                                                                                                   |
| `close_period`       | duration             | as in `Constants.close_period`                                                                                                                                      |
| `confirmation_depth` | integer              | block depth the provider requires before treating a UTxO as settled — this binding's confirmation policy                                                            |
| `min_iou_amount`     | integer              | minimum amount the first IOU must carry; dust prevention                                                                                                            |
| `catalog`            | resource → price map | pricing per resource                                                                                                                                                |
| `reference_script`   | output ref, optional | tx-building convenience                                                                                                                                             |

### 5.2 Request payload (client → server)

```
Request  = ById(channel_id: (iou_key, tag), commit: Commit)
         | ByPopMac(auth: PopOrMac, commit: Commit?)
Commit   = Iou(amount: Int, sig: Signature) | Mutual(transaction: Bytes)
```

`ByPopMac` lets a client prove liveness (and, on the first such call, earn a session MAC for
cheaper follow-ups) without repeating a full channel-identifying IOU on every request.

### 5.3 Response payload (server → client)

```
Response = { success: Bool, error: CoreError?, status: Status }
Status   = { channel: ChannelStatus?, mac: MacTtl? }
ChannelStatus = { iou: (amount, sig), spendable: Int?, uncommitted: Int? }
```

`spendable = None` means no backing channel was found at all (no matching UTxO, bad datum,
insufficient confirmation depth, or the provider otherwise treats it as inactive). `uncommitted`
is backing observed but not yet confirmed to `confirmation_depth`.

## 6. Verification rules

A request's commit is valid only if, against the channel the provider is currently tracking for
this `(iou_key, tag)`:

- the IOU's signature verifies under `iou_key` over `(tag, amount)`;
- `amount` is strictly greater than every amount already committed for this channel (an IOU that
  isn't is rejected, not merely ignored — see `CoreError::OldIou`);
- `amount` does not exceed what the channel can actually back once claimed (`CoreError::Unspendable`);
- the channel itself currently backs at least `confirmation_depth` blocks of finality
  (`CoreError::NoChannel`/`Inactive` otherwise).

### 6.1 Error code mapping

`CoreError` is Subbit's own vocabulary. Mapped onto x402's §9 `errorReason` codes, reusing a
generic code wherever one genuinely fits and following the EVM batch-settlement binding's own
precedent (`invalid_batch_settlement_evm_cumulative_amount_mismatch`) of a scheme-specific code
where none does:

| `CoreError`                                                  | `errorReason`                                                              |
| ------------------------------------------------------------ | -------------------------------------------------------------------------- |
| `Unspendable`                                                | `insufficient_funds`                                                       |
| `OldIou`                                                     | `invalid_batch_settlement_cardano_cumulative_amount_mismatch`              |
| `NoChannel`, `NoIou`, `OldAuth`, `InvalidAuth`, `InvalidIou` | `invalid_payload`                                                          |
| `Inactive`                                                   | `invalid_batch_settlement_cardano_channel_inactive`                        |
| `MutualNotSupported`                                         | `invalid_batch_settlement_cardano_mutual_unsupported`                      |
| `RateLimited`                                                | not an x402 error at all — surface as plain HTTP 429, outside the envelope |
| `Other`                                                      | `unexpected_verify_error`                                                  |

`NoChannel`/`NoIou`/`OldAuth`/`InvalidAuth`/`InvalidIou` collapse to one generic code because the
distinctions between them (no backing UTxO vs. no IOU yet vs. a bad signature) aren't ones a
generic x402 client acts on differently; a human-readable `invalidMessage` carries the detail
instead, per §9's own guidance not to key behavior off message text.

## 7. Storage behavior

The provider keeps a per-channel ledger of the latest accepted IOU amount (`subbed`-equivalent,
tracked off-chain ahead of any on-chain claim) and classifies funds as `spendable` (confirmed
backing) or `uncommitted` (observed, not yet confirmed). The commitment identifier on success is
`Keytag` (§4.3); storage is keyed by it, with the multiplicity caveat already noted.

## 8. Double-spend prevention

Two checks, both ordinary consequences of the cumulative-IOU mechanic (§4.3):

- **Supersession.** An old, lower-amount IOU is simply superseded by a newer one; presenting it
  after a higher amount is already accepted is rejected outright (`OldIou`).
- **On-chain ceiling.** `Sub`/`Settle` both require `n ≥ subbed_out` directly in script, so a
  claim can never exceed the signed cumulative amount.

`Mutual` relies on neither — both parties must sign it directly (§4.2).

## 9. Commitment expiry

`Close` (consumer-signed) fixes `elapse_at` to at least the closing transaction's own validity
upper bound plus `close_period`, and freezes `subbed`. Between `Close` and `elapse_at`:

- the provider may `Settle` once, for up to the full latest signed IOU amount minus what's
  already been subbed;
- the consumer may not touch the channel at all.

Once the validity lower bound reaches `elapse_at`, the consumer may `Elapse`, unconditionally
reclaiming whatever the provider left unclaimed. This is the capital-backed loss bound: a
provider that never settles forfeits only the value above its last accepted `Sub`, within one
`close_period` window — never more.

## 10. Redemption

`Sub` and `Settle` are both triggered **solely by the provider**, at its own discretion, by
building and signing a Cardano transaction directly against the validator. There is no other
rail: no facilitator broadcast, no delegated claim service. `Sub` happens during normal operation
(channel stays `Opened`); `Settle` is the provider's last chance, available only after the
consumer has `Close`d and only until `elapse_at`. Many channels may be redeemed in a single
transaction via the `Main`/`Defer` batching mechanism (§4.2).

`Mutual` is the cooperative exit: both parties sign one transaction, built and sent by the
consumer, co-signed by the provider. The validator checks only that both signed and that the
spend is solo (no other channel input alongside it) — see §8 above and §11 below for what this
does and doesn't guarantee.

## 11. Chain-watching, and why this binding defines no facilitator-managed mode

Every provider is solely responsible for watching the chain for its own channels: knowing which
UTxOs back which `(iou_key, tag)` pairs, to what confirmation depth, and whether a channel has
closed. Nothing in this binding delegates that to anyone else, and that omission is structural,
not a missing feature to add later.

The reason: claiming funds (`Sub`, `Settle`) _is_ producing the provider's own required-signer
witness, and the destination is whatever output that signing transaction constructs — the
validator fixes no payout address. A party able to produce that signature can send the channel's
funds anywhere, so performing `Sub`/`Settle` on a provider's behalf requires holding its key over
its channels: full custody, not delegation.

A validator that fixed the receiver address in the datum — paid by every claiming step
regardless of who signs, the way an account-based channel contract fixes `receiver` once at
creation — could support a facilitator-managed mode safely. This one doesn't, so this binding
doesn't define one.

## 12. Security considerations

- **Reorg risk** is bounded by `confirmation_depth`; a provider accepting IOUs against a UTxO
  shallower than its own configured depth is accepting rollback risk it chose to carry.
- **Keytag collisions** (§4.3) mean a provider's lookup by `(iou_key, tag)` can return more than
  one `Opened` channel; providers MUST have a deterministic selection rule (e.g. prefer the
  candidate with outstanding uncommitted value, tie-broken by lowest `subbed`) and MUST be able
  to handle more than one match safely rather than assuming at most one.
- **Dust channels** are bounded by `min_iou_amount` on the opening IOU, advertised in `Terms`.
- **Batching correctness** depends on exactly one `Main` redeemer per transaction, on the
  lexicographically-first channel input; every other channel input MUST carry `Defer`, enforced
  by the validator itself, not by off-chain convention.
- **Native-asset spam.** A continuing output's value is checked strictly (`amount_out`): it MUST
  carry exactly the channel's declared asset and nothing else, preventing a spend that smuggles
  unrelated tokens into a continuing output.
- **`Mutual`'s value split is off-chain, and co-signing is broader than it looks.** The script
  checks only that both parties signed and the spend is solo. But signing a transaction
  authorizes spending _every_ input it carries that's under the signer's credential, not just
  the channel UTxO being closed — so before co-signing, the provider must check the whole
  transaction: that the first output pays its unclaimed amount to its own address, and that no
  other input belongs to it. Co-signing without that check can authorize spending UTxOs at the
  provider's own address that have nothing to do with this channel.

## 13. Test vectors

IOU, from this binding's own reference test fixtures:

```
tag      = 7375626269742d746573742d746167   (ASCII "subbit-test-tag")
iou_key  = 8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c
amount   = 1000
message  = cbor.serialise((tag, amount))   ; indefinite-length Plutus Data list
signature (Ed25519, over `message`):
  104e39b968bb917eff7ed097c3b0760d04cdf13cc54f4b89f4d0dc844baf4ea
  481af64210d615fe6ebff9e2626f177b7063af6a90980e6ffaceced6f4804fd07
```

`Keytag` for this fixture is `iou_key ‖ tag` (concatenated raw bytes, not hashed).

## 14. Open items

- **Error code mapping.** The `errorReason` field is already wired (§6) but carries `CoreError`'s
  own `Display` strings verbatim, not x402's §9 vocabulary — see §6.1 for the proposed mapping.
- **`payment_flow` declaration.** §5.1 now specifies `"upfront"`; not yet implemented in
  `KnownExtra::SubbitV1`.
- **Facilitator-managed mode.** Deliberately undefined; see §11. Revisit only if a validator
  variant exists that fixes the payout address independent of the claiming signer.
