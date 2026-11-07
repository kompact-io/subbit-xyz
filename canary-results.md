# canary results

`packages/canary` drives a full subbit channel lifecycle (Open -> Add -> Close ->
Settle/Elapse -> End) against real Cardano preprod via Floe, through
`subbit-console`'s `Ctx`/`Wallet` API (the same engine the CLI and wasm UI use),
and doubles as the L2 throughput benchmark for `subbit-gate`.

Limitations: Canary runs consumers L2 via `subbit-stamp` on the same host as `subbit-gate`.
Thus roundtrip times are unrealistically short for real world comparison.
However it does mean that \~1/3 of the CPU is spent on consumer side logic.

WARNING: Run it with `--release` (debug-mode signing is \~50-60x slower and not representative), needs
`WALLET_SKEY`/`L2_SEED_HEX` in the environment. See `canary --help` for flags.

## Headline numbers (current, trustworthy, real preprod)

- **L2 steady-state throughput:** 23,000-28,000 req/s (40 consumers, mixed
  vanilla/x402).
- **L2 latency:** p50 \~1.4ms, p90 \~2.1ms, p99 \~2.9ms.
- **Firehose ceiling** (no gate business logic, cached-`Mac` liveness proof):
  \~175,000-210,000 req/s -- matches the bare-axum echo baseline.
- **Fully-authenticated floor** (`always_by_id`, no liveness amortization,
  every request independently verified): \~50,000-51,000 req/s.
- **L1 fees** (Blockfrost-confirmed, 200-consumer run): Open \~11,100-13,800
  lovelace/channel; Add/Close (consumer-signed) \~38,700-47,100 lovelace/channel
  -- \~3-4x Open, the cost of the extra per-consumer witness.
- **Correctness:** every verified run above completes the full lifecycle against
  real preprod with `spent == committed` exactly.

## Problems found and fixed

One line each (kept for historical purposes)

- Reference-script lookup searched the wrong address (validator's, not the
  funding wallet's) -- `canary/src/chain.rs`.
- Fuel-input spending never got an unconditional wallet witness, only a
  fallback inside the signer loop -- `console/src/tx/submit.rs`.
- A submitted-but-unconfirmed tx raced the next round because `submit` only
  waited for confirmation on multi-batch `Open`, not every call -- same file.
- A transient backend read error during landing-poll aborted the wait instead
  of retrying -- same file.
- Burst-mode round throughput was computed off the L1 chain step's timer, not
  the L2 firehose's own elapsed time -- off by \~6.6x. Invalidated two prior
  figures in this history (both since re-measured correctly) -- `canary/src/orchestrator.rs`.
- `subbit-gate`'s flush/checkpoint thresholds were sized for lovelace, not ADA,
  causing excessive `redb` commits under load; rescaled -- \~30% latency drop
  (`canary/src/provider.rs`, `gate/src/db.rs` defaults).
- Firehose benchmark re-signed a fresh `Pop` every request instead of reusing a
  cached `Mac` -- \~57k -> 175k req/s once fixed (`gate/benches/firehose.rs`).
- `subbit_stamp::Stamp` had the same bug in a different shape (unconditional
  `verify_iou`); fixed to only verify when the echoed amount increases.
- x402 dialect was missing vanilla's mac-invalidation reaction on `InvalidAuth`;
  added, plus a parity test.
- `subbit-gate`'s `spawn_blocking` thread pool was unbounded (200-250 threads
  under load); bounded via a semaphore -- p99 19-21ms -> 7.7-8.3ms.
- Canary's own latency instrumentation serialized every request on one
  `Mutex<Vec<Duration>>`, undercounting throughput \~30%; fixed with per-task
  accumulation.
- `:burst` personas all raced to `Close` on round 0 (round-indexed, not
  time-indexed), skewing live concurrency down over a run; fixed with a fixed
  `close_after` from each consumer's own construction time.
- `MAX_LOOP_TIME` didn't track `close_upper`'s margin when the latter was
  bumped, so runs timed out before `Elapse` could land; raised 900s -> 2400s.

## Resolved: "consumer-signed batched step sometimes never lands"

Reproduced twice pre-fix (confirmed via Blockfrost 404, not indexer lag).
Leading theory was an underfee margin never ported from canary's pre-refactor
flat 150% multiplier into the shared engine. Re-ran the identical shape (40-way
simultaneous `Close`) after Floe's 2026-11-05 stalling-bug fix: zero issues,
multiple times. Treat as resolved; if it resurfaces, check the fee-margin theory
next.

## Still open

- `subbit-gate`'s commit-phase cost under canary's concurrent topology (\~350us)
  is higher than the isolated `verify_iou` cost (\~33-105us) -- not explained.
- Canary's own round-trip latency is a stable \~2.5x the firehose's at equal
  concurrency -- not isolated to client vs. server vs. network.
- Even with the round-0 race fixed, \~half of 40 simultaneous `Close` proposals
  still take many extra rounds to land -- a `console::submit` batching/retry
  constraint, not chased further.
