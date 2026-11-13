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

## Latest run: severity ladder 1 -> 4 -> 40 -> 200 (2026-11-12)

Full ladder against real preprod, after fixing `subbit-stamp`'s ById accounting,
the vanilla dialect's resync-on-rejection, `subbit-gate`'s write-buffer data
loss, and -- the headline fix -- gate finally bounding every accepted IOU by
on-chain backing (previously `spent == committed` was the only check; a run
could accept several times the actual locked funds). See appendix for one-line
summaries of each.

**L2 throughput: solid.** Peak observed **~24,700 req/s** at 200 concurrent
consumers (round-to-round delta over real wall-clock gaps between round
reports -- canary no longer logs live req/s directly, see appendix). This
tracks closely with the **~27,000-30,000 req/s** ceiling previously measured
at 40 consumers: throughput looks gate-bound in that range, not
consumer-count-bound. Every active round's spend count matched the expected
monotonic growth exactly, and every severity's `spent == committed` held:

| severity | total spend = committed                                      | notes                                                                                                      |
| -------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| 1        | 23,045,600                                                   | full lifecycle, clean                                                                                      |
| 4        | 97,836,700                                                   | full lifecycle, clean (first attempt hit a since-fixed stall, see appendix)                                |
| 40       | n/a                                                          | manually concluded mid-run once wallet funds were reported depleted, not run to its own natural completion |
| 200      | 40,074,449 (final, round 4 onward -- deposits fully drained) | see below -- functionally complete, but the orchestrator itself mis-reported failure                       |

**L1: 200-channel batching confirmed, including same-block settlement for the
full set.** The batched `Open` landed all 200 channels in one transaction, and
multiple `subbit-claim` `Sub` batches bundled many channels' commits into a
single transaction each -- consistent with observing all 200 channels subbed
in a single block during this run. Illustrative transaction ids captured this
session (preprod; not independently re-verified against an explorer in this
session, so treat the block/channel-count detail as the observation that
prompted this run rather than a re-confirmed fact -- follow the links to check
directly):

- `https://preprod.cexplorer.io/tx/1b4f80d395a9412bdea7d5663f15086bccd1203147ef829bee801759346b9073`
- `https://preprod.cexplorer.io/tx/043242efcf8c1fad38931baeb17cb67a9afe6ede2fa9e6a15114d7c34ca4bac3`
- `https://preprod.cexplorer.io/tx/75ed7eeb84dd5fce6a100354f07a50ea512dfedb8fcb1817427b682b1503723f`

**Severity-200's actual outcome:** all 4 active rounds completed correctly
(40,074,449 total spends, stable from round 2 onward as every consumer's
deposit drained -- expected now that gate bounds spends by backing). Tidying
(Close/Elapse/Settle across 200 channels) then ran past `MAX_LOOP_TIME`
(2400s) and the run exited with `Error: lifecycle loop exceeded 2400s without
every channel completing`. A `canary tidy`/`canary status` run immediately
afterward found **zero remaining channels and nothing left in the claim
wallet** -- every channel had, in fact, already fully resolved on-chain by
the time the error fired. This points to a check-ordering issue in
`Orchestrator::run_until_done` (`packages/canary/src/orchestrator.rs`): the
`MAX_LOOP_TIME` check runs at the top of the loop, before that pass's own
`refresh`/`live.is_empty()` check -- so a run that would have reported success
on its very next tick can instead report failure one tick early. Not yet
fixed; see "Still to do" below.

### Fixes shipped this session

- Four lifecycle/correctness fixes (`subbit-stamp` ById mint accounting,
  vanilla dialect resync-on-rejection, gate write-buffer data loss, gate
  backing-bound enforcement) -- one line each in the appendix.
- `packages/canary/src/orchestrator.rs`: added `CHAIN_STEP_TIMEOUT` so a
  hung `Open`/`flush` fails fast instead of blocking the whole run silently
  until `MAX_LOOP_TIME`. Tuned from an initial 120s (too aggressive -- fired
  on a 40-consumer `Open` that had actually landed fine, just slower than
  120s to confirm) up to 900s, clear of the documented 790-845s historical
  landing-time figure at 40 consumers.
- `packages/claim/src/pending.rs` + `src/main.rs`: `subbit-claim`'s
  rebroadcast loop now backs off (doubling, capped at 32x the base interval)
  when amaru rejects a rebroadcast as a duplicate, instead of hammering the
  same already-landed-or-pending tx every tick for however long
  confirmation lag lasts (observed live: several minutes, repeatedly).

### Still to do

- Fix the `MAX_LOOP_TIME`-vs-`live.is_empty()` check-ordering race described
  above (false failure on a run that actually finished).
- Canary has no signal handler, so a plain `kill`/SIGTERM on its top-level
  pid orphans its spawned `subbit-gate`/`lineage-index`/`subbit-claim`
  children -- `kill_on_drop(true)` is already set on all three
  (`packages/canary/src/process.rs`) but never fires because a raw signal
  skips Rust destructors entirely. Needs a real `tokio::signal` handler.
- Surface L2-op metrics gate-side (IOU count, total amount, req/s) instead
  of only via canary's after-the-fact round deltas. Must not be a single
  shared mutex around a counter -- gate is on the hot path at the
  20-30k req/s range seen here, and a shared lock would itself become the
  bottleneck. Needs a design pass (per-worker/sharded counters, atomics, or
  sampling).
- **Not scheduled yet:** a 400-channel run, to see whether that saturates
  blocks the way 200 channels appears to (per this session's "channels
  subbed in a single block" observation) or whether 200 was already at or
  near the batching ceiling.

## Appendix: historical numbers and learnings (terse)

**Headline numbers, pre-this-session (still broadly representative):**

- L2 steady-state throughput: 23,000-28,000 req/s (40 consumers, mixed
  vanilla/x402) -- consistent with this session's 200-consumer peak (~24,700
  req/s).
- L2 latency: p50 ~1.4ms, p90 ~2.1ms, p99 ~2.9ms.
- Firehose ceiling (no gate business logic, cached-`Mac` liveness proof):
  ~175,000-210,000 req/s.
- Fully-authenticated floor (`always_by_id`, no liveness amortization):
  ~50,000-51,000 req/s.
- L1 fees (Blockfrost-confirmed, 200-consumer run, pre-this-session): Open
  ~11,100-13,800 lovelace/channel; Add/Close (consumer-signed)
  ~38,700-47,100 lovelace/channel.
- Pre-this-session correctness note (now superseded): `spent == committed`
  was treated as sufficient on its own -- this session's backing fix showed
  that check alone let a run accept several times the actual locked funds.

**Problems found and fixed, pre-this-session (one line each):**

- Reference-script lookup searched the wrong address (validator's, not the
  funding wallet's).
- Fuel-input spending never got an unconditional wallet witness.
- A submitted-but-unconfirmed tx raced the next round (only multi-batch
  `Open` waited for confirmation, not every call).
- A transient backend read error during landing-poll aborted the wait
  instead of retrying.
- Burst-mode round throughput was computed off the L1 chain step's timer,
  not the L2 firehose's own elapsed time -- off by ~6.6x.
- `subbit-gate`'s flush/checkpoint thresholds were sized for lovelace, not
  ADA -- ~30% latency drop once rescaled.
- Firehose benchmark re-signed a fresh `Pop` every request instead of
  reusing a cached `Mac` -- ~57k -> 175k req/s once fixed.
- `subbit_stamp::Stamp` had the same unconditional-`verify_iou` bug in a
  different shape; fixed to only verify when the echoed amount increases.
- x402 dialect was missing vanilla's mac-invalidation reaction on
  `InvalidAuth`.
- `subbit-gate`'s `spawn_blocking` thread pool was unbounded -- bounded via
  a semaphore, p99 19-21ms -> 7.7-8.3ms.
- Canary's own latency instrumentation serialized every request on one
  `Mutex<Vec<Duration>>`, undercounting throughput ~30%.
- `:burst` personas all raced to `Close` on round 0 (round-indexed, not
  time-indexed), skewing live concurrency down over a run.
- `MAX_LOOP_TIME` didn't track `close_upper`'s margin when the latter was
  bumped, so runs timed out before `Elapse` could land; raised 900s -> 2400s
  (the same constant this session's 200-run then hit again at 200-channel
  tidying scale -- see "Still to do").

**This session's fixes (one line each, detail above):**

- `subbit-stamp` ById mint accounting (`Stamp::commit`): a resync could land
  the next mint exactly on `committed`, which gate rejects as `OldIou`
  forever; now mints strictly above it.
- Vanilla dialect adopts the channel status a rejection carries, for every
  error except `NoChannel`/`Inactive`/`Other`.
- Gate's write-buffer `take` carried forward entries from a failed-commit
  generation instead of dropping them (root cause of a `NoChannel`-forever
  bug).
- Gate now bounds every accepted IOU by on-chain backing
  (`Backing::backs`); a rejected IOU is not recorded. Before this, every run
  accepted several times the locked funds.
- `CHAIN_STEP_TIMEOUT` added to the orchestrator (see above).
- `subbit-claim` rebroadcast backoff on a duplicate rejection (see above).

**Resolved (carried forward):** "consumer-signed batched step sometimes
never lands" -- reproduced twice pre-fix, resolved after Floe's 2026-11-05
stalling-bug fix; zero recurrences since, across every severity run this
session included.

**Still open, pre-this-session (unchanged, not chased further):**

- `subbit-gate`'s commit-phase cost under canary's concurrent topology
  (~350us) is higher than the isolated `verify_iou` cost (~33-105us) -- not
  explained.
- Canary's own round-trip latency is a stable ~2.5x the firehose's at equal
  concurrency -- not isolated to client vs. server vs. network.
- Even with the round-0 race fixed, ~half of 40 simultaneous `Close`
  proposals still take many extra rounds to land -- a `console::submit`
  batching/retry constraint, not chased further. (This session's
  `subbit-console::tx::submit` stale-intent retry loop is the same
  underlying gap: still has no cap/backoff on the Open/non-Will path.)
