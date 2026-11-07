# canary results

`packages/canary` drives a full subbit channel lifecycle (Open -> Add ->
Close -> Settle/Elapse -> End) against real Cardano preprod via Floe,
through `subbit-console`'s `Ctx`/`Wallet` API only (no direct
`cardano-connector`/`cardano-wallet`/`cardano-session`/`amaru-*` use on
the consumer side any more), and doubles as the L2 throughput benchmark
for `subbit-gate`.

## Architecture (post-refactor)

Every consumer-side L1 action (keyring, staging, batching, signing,
submission, confirmation-wait, retry/backoff) now goes through
`subbit_console::{Ctx, submit, upload_reference_script, send_payment}`
over a `subbit_wallet::Embedded<floe_client::grpc::Client>` wallet --
the same shared engine the CLI and wasm UI use, not a canary-specific
reimplementation (`packages/canary/src/chain.rs` is just the thin
translation layer between canary's own per-round questions and that
API). `--mode mock` and `--mimic` are both gone entirely: there is no
in-memory/UPLC-eval chain any more, and no decoy-channel scenario in
the normal run path. Provider-side (`subbit-gate`, `lineage-index`,
`subbit-claim`) is spun up as real, independent spawned binaries;
canary makes no further direct calls into their internals after that
("spin up, walk away").

Legacy deps (`cardano-connector`, `cardano-wallet`, `cardano-session`,
`amaru-uplc`/`amaru-plutus`/`amaru-kernel`) are confirmed fully removed
from `subbit-canary` via `cargo tree -p subbit-canary -i <crate>`.

## Running it

```
canary --persona alice=46
canary --status --scan-consumers 60
canary --tidy --scan-consumers 60
canary --persona alice=46 --resume 4
canary --floe-endpoint https://grpc.floe.kompact.io
```

Needs `WALLET_SKEY`, `L2_SEED_HEX` in the environment (`.env` at the
repo root -- `WALLET_SKEY` is a real preprod signing key with real
(testnet) funds; never commit `.env`, never print the key beyond what's
operationally necessary).

**Always run `--release`.** Debug-mode elliptic-curve signing measured
~50-60x slower than release in this codebase -- a debug run's numbers
have no relationship to real throughput. Canary itself warns loudly on
startup if built in debug mode. The spawned `lineage-index`/
`subbit-claim` binaries also need a release build
(`cargo build --release --bin lineage-index --bin subbit-claim`) --
canary looks for them as siblings of its own executable, so a
debug-only canary binary silently can't find a release-only sibling
and vice versa.

## Bugs found and fixed this session (live preprod, post-refactor)

All four surfaced only once real consumer-signed traffic ran against
real preprod -- the mock chain never would have caught any of them.

1. **Reference-script discovery searched the wrong address.**
   `Chain::refresh` only ever searched the _validator's_ address for
   the reference-script utxo, but `upload_reference_script`/
   `send_payment`'s shared builder puts it at the _wallet's own_
   change address (same place the old `cardano-wallet`-based code
   always did). Fixed: `refresh` now also searches the funding
   wallet's own pay-hash and looks for the script-bearing utxo there.

2. **Missing unconditional wallet witness for fuel inputs.**
   `console::submit` only called `wallet.sign_tx` as a fallback inside
   the `specified_signatories` loop -- it never unconditionally
   obtained the funding wallet's own witness for the ordinary
   input-spending authorization it owes as the fuel-UTxO owner,
   independent of any validator-level required signer. First real
   `Open` attempt failed synchronously: `missing required signatures
for keys or roots: [c6446bd5...]`. Fixed: added an unconditional
   `wallet.sign_tx`/`add_witness` call after the loop (idempotent if
   the loop already covered it).

3. **A submitted-but-unconfirmed tx raced the very next round.**
   `console::submit`'s success path only called `wait_for_landing`
   when more `Open` batches remained -- a `Will`-only submission
   (`Add`, `Close`) returned the instant the wallet reported mempool
   acceptance. `chain.rs`'s `flush()` then immediately `refresh()`'d
   and the orchestrator's next round built its decision off state that
   hadn't confirmed yet, risking a channel `Input` reference that was
   already consumed by the still-unconfirmed previous step. Fixed:
   `submit` now unconditionally waits for confirmation before
   returning, for every caller (CLI, wasm, canary), not just
   multi-batch `Open` draining.

4. **`wait_for_landing` treated a transient backend error as fatal.**
   Hit live: `floe error: ... "index is still in catch-up mode; only
by-ref lookups are available"` during a landing-poll, which
   immediately aborted the whole wait via `?` instead of retrying.
   Fixed: a read error during the poll is now treated the same as
   "not found yet" and the poll keeps backing off, rather than bailing.

All four are fixed in `packages/console/src/tx/submit.rs` (3, 4) and
`packages/canary/src/chain.rs` (1, 2) respectively; `cargo test -p
subbit-console` (63 tests) and `cargo check -p subbit-canary` both
clean after each fix.

## Open issue: a consumer-signed batched step sometimes never lands, no error surfaced

Reproduced twice independently, both confirmed via Blockfrost
(`GET /txs/{hash}` -> 404, well past any plausible indexer lag):

- Single-channel run (`alice=1`): the `Close` step's own tx hash
  (`c8d2c4383d8df5f1e013ee20d94768f638fc0335828d9cf068729099d9c206ea`)
  was accepted by `wallet.submit` (`Ok`), `wait_for_landing` polled 20
  times (~5min, exponential backoff capped at 30s) and gave up; never
  appeared on Blockfrost afterward either.
- 40-channel release run (`alice=40`), round 2's 27-want batch: tx
  `2a56fe0057bed63bbce3f321d21cd7b9af673bca015fd7fc9e7033dd64c2f38c`,
  same shape -- accepted, never confirmed, no rejection ever surfaced
  back to the caller.

In both cases the _preceding_ step (an `Open` or `Add`) landed and
confirmed correctly first -- ruled out the race in fix #3 above as the
cause here; this is something else.

Live preprod `min_fee_a`/`min_fee_b` (`44`/`155381`, via Blockfrost
`/epochs/latest/parameters`) still match `cardano_sdk::
ProtocolParameters::from(Network::Preprod)`'s hardcoded snapshot
exactly, same as the pre-refactor investigation found -- so this isn't
a stale linear-fee-constant problem in the obvious sense. Strong
suspect, carried over from before the refactor and still unresolved:
the pre-refactor `results.md` documented the _same symptom_ on a plain
reference-script upload (~15% underfee, root cause "not fully chased
down") and worked around it with a flat 150% fee-margin multiplier --
explicitly scoped to canary's own tx-building only, _not_ applied to
the shared engine production code (`subbit-claim`) uses. That margin
was never ported into `subbit_tx::NetworkParameters::for_network`
(which `console::{submit,upload_reference_script,send_payment}` all
use internally) when canary moved onto it -- `chain.rs`'s own module
doc already flags this as a known, deliberately-unfixed gap, reasoning
that baking a canary-only guess into every console caller (including
real `subbit-claim` production operation) risked changing behavior
everywhere on an unmeasured margin. Both repro cases above are
consistent with that same underestimate reappearing, now for
consumer-signed `Add`/`Close` steps rather than a plain upload --
matching the pre-refactor fee table's own finding that `Add`/`Sub`
costs ~3-4x `Open` per channel (the extra per-consumer witness, not
the step logic), i.e. exactly the kind of tx where a script-execution-
cost underestimate would bite hardest. Not yet fixed: needs either a
real measured margin (not a guess) or tracking down why the estimated
exec units/fee for these specific steps comes in short, before this
can be called resolved rather than papered over again.

## L2 latency vs. flush frequency (2026-11-06)

Followed up on the flush-frequency theory above with a real change, not
just analysis: `packages/gate/src/db.rs`'s defaults (`BufferConfig`:
`max_pending=2_000`, `max_buffer_interval=50ms`; `ExposureConfig`:
`single_write_threshold=5`, `max_cumulative_exposure=500`,
`max_checkpoint_interval=30s`) are tiny relative to real ADA amounts --
at canary's flat-cost of 100 lovelace/request, the _size_ trigger fires
every ~20 requests, not the time trigger, meaning thousands of
`redb` write-transaction commits (~700us each, serialized behind one
`flush_lock`) per second under load. Bumped canary's own spawned-gate
config (`packages/canary/src/provider.rs::spawn_demo`) to real-ADA
scale -- `buffer.max_pending`=10 ADA, `exposure.single_write_threshold`
=1 ADA, `exposure.max_cumulative_exposure`=10 ADA, both
`max_buffer_interval`/`max_checkpoint_interval`=1s -- so the 1s timer
dominates instead.

Also removed `Persona::burst`'s hardcoded 10s deadline (now a 10-year
"deadline" so `commit_max` is the only stop condition -- `Instant +
Duration::MAX` panics, so not literally `MAX`), and added
`--burst-target-lovelace` so a `:burst` persona can commit to a fixed
lovelace amount instead of its full deposit.

**Result (`alice:burst=40`, `--flat-cost 27 --burst-target-lovelace
1000000 --no-elapse`, bumped flush config):**

```
L2 latency -- 1,481,520 spend_burst round-trip(s), p50=1.26ms p90=2.04ms p99=2.86ms
```

vs. the pre-bump baseline (`richy:burst=40`, flat-cost 100, default
flush config, from the open-issue re-run above):

```
p50=1.78ms p90=3.04ms p99=4.08ms
```

Roughly a 30% drop across every percentile. Consistent with the
flush-frequency theory -- fewer, larger flushes under less lock
contention measurably helped. Not a clean single-variable ablation
though: persona (richy->alice) and flat-cost (100->27) also changed in
the same run, so some of the delta could be from request/commit
pattern differences rather than the flush config alone. Worth one more
run holding persona/flat-cost fixed and only toggling the flush config,
before calling this confirmed.

**Timed fixed-transfer side result:** sizing a burst to transfer a
fixed 1 ADA per consumer in 27-lovelace increments (chosen to target
~60s given a conservative 625 req/s/consumer estimate) actually
completed in 31.5s (46,988.9 req/s aggregate across 40 consumers) --
real throughput ran ~2x the sizing estimate. `spent == committed`
exactly (40,001,040 total, avg 1,000,026/consumer -- the small
overshoot is the last 27-lovelace step crossing the 1,000,000
threshold, expected with a fixed increment not dividing the target
evenly).

> **Correction (2026-11-07):** the "31.5s / 46,988.9 req/s" figure above was
> measured with the same bug described in "Burst-mode round-throughput
> measurement bug" below -- it's the L1 chain step's elapsed time, not the L2
> firehose's. Treat it as unreliable; `spent == committed` and the lifecycle
> completion itself are unaffected. No corrected replacement number for this
> specific flat-cost=27 configuration; see the fixed-measurement run below for
> a trustworthy figure at flat-cost=14.

## Burst-mode round-throughput measurement bug, found and fixed (2026-11-07)

Running `alice:burst=40 --flat-cost 14 --burst-target-lovelace 1000000 --no-elapse` (sized to
target roughly a minute-long burst) first surfaced a round-report throughput of 186,028.9 req/s
-- implausible on its face: right at the stack's documented hard ceiling (170-210k req/s echo
baseline), which canary itself had never previously reached, sitting instead at a stable ~2.5x
gap below it (an open issue above). Sanity check via Little's Law
(`concurrency / per-request-latency ~= throughput`; 40 consumers, each running `spend_burst`
strictly sequentially -- one request in flight per consumer, confirmed in `consumer.rs`): the
run's own independently-measured latency (p50=1.43ms) implies ~28,000 req/s, not 186,029 -- a
~6.6x mismatch between the round-level figure and what the run's own latency says it should be.

**Root cause** (`packages/canary/src/orchestrator.rs`): the round-report line's throughput was
computed from `chain_elapsed` -- the L1 chain step's own timer (`want`/`flush`) -- not from how
long the L2 firehose task actually ran. For a continuous (non-burst) persona the two spans
coincide by construction (the firehose only stops once told to, right after the chain step
finishes), so this went unnoticed for every prior non-burst measurement in this document. A
`:burst` persona's `firehose_l2` (`spend_burst` in `consumer.rs`) never checks that stop signal
at all; it runs to its own `commit_max`/`deadline` on a timeline unrelated to the chain step's.
The printed "req/s" for a burst round was therefore dividing by the wrong elapsed time outright,
not just adding noise. Likely also corrupts the "Timed fixed-transfer side result" entry above
(flat-cost=27, "31.5s / 46,988.9 req/s") -- same code path, see the correction note added there.
Latency percentiles throughout this document are unaffected: they come from `timed_spend`'s own
independent `Instant`/`elapsed()`, never from `chain_elapsed`.

**Fix:** the firehose task now gets its own `Instant` (`firehose_start`, taken before it's
spawned), and the round report uses that elapsed time (`spend_elapsed`) for the req/s
calculation, with `chain_elapsed` printed alongside it rather than silently substituted in.

**Re-run after the fix, same scenario, twice for reproducibility** (exposure config unchanged:
`buffer.max_pending`=10 ADA, `exposure.single_write_threshold`=1 ADA,
`exposure.max_cumulative_exposure`=10 ADA, both intervals=1s):

```
run 1: round 0: 2,857,160 spend request(s) in 121.703639838s (23,476.4 req/s), 40 chain step(s) submitted in 121.666094820s
run 2: round 0: 2,857,160 spend request(s) in 102.800169512s ( 27,793.3 req/s), 40 chain step(s) submitted in   7.342126544s
```

Both completed the full lifecycle cleanly (`spent == committed` exactly, 40,000,240 total, each
run). Run 2 is the clean confirmation that the two clocks are now genuinely independent: its
chain step (the `Close` batch) landed in 7.3s, well before the burst itself finished 95s later --
something the old code could never have represented, since it only ever had one timer to report.

Latency, both runs (consistent with every other measurement in this document):

```
run 1: p50=1.415379ms p90=2.118393ms p99=2.911226ms
run 2: p50=1.392876ms p90=2.109823ms p99=2.916508ms
```

23,476-27,793 req/s is consistent with this document's other `alice`-roster numbers (20-30k
steady-state) and with the Little's-Law estimate above -- this is now a trustworthy figure, not
an artifact.

## Open issue re-run: `richy:burst=40`, post floe-stall fix (2026-11-06) -- did not reproduce

Re-ran the exact shape that hit the "never lands" symptom above
(simultaneous multi-consumer `Close` batching), now against the
redeployed floe (fixed 2026-11-05 for a stalling bug -- see its own
note). `canary --persona richy:burst=40` (33 x402 + 7 vanilla
consumers, release build):

- Full lifecycle completed clean: `spent == committed` exactly
  (82,231,200 total across all 40 consumers).
- Zero errors, panics, rejections, or stalled/unconfirmed txs anywhere
  in the run -- including the round-85 40-way simultaneous `Close`
  batch, i.e. exactly the shape that previously got stuck. No
  `MaxTxSizeUTxO` or other underfee-shaped rejection appeared either.
- Only one transient, self-resolved `subbit_claim` "waiting for tx
  confirmation" retry log line -- benign, not a repeat of the open
  issue.
- L2 latency: 822,312 round-trips, p50=1.78ms p90=3.04ms p99=4.08ms --
  in line with the prior session's canary-latency numbers.

This is one clean run, not a disproof, but it's consistent with the
floe-stalling-fix theory (floe redeployed 2026-11-05) being the actual
cause of the "never lands" symptom above, rather than the fee-
underestimate theory this doc previously leaned toward -- no underfee
rejection showed up at all here, where the fee theory would predict
one. Worth another burst run or two to build confidence before
downgrading the open issue to resolved.

## 40-channel release-build stress run (`alice=40`, this session)

Resumed from a single already-open channel; 39 fresh `Open`s batched
into one tx (all landed, confirmed via `--status` on the next run
showing all 40 recognized). `subbit-gate` (real spawned binary, not
in-process) was correctly the CPU workhorse throughout (~270-345%),
`canary`'s own firehose driver close behind (~240-300%), `subbit-claim`
correctly near-idle (~0.1%) since claiming is genuinely just its own
async poll loop with nothing yet to do.

```
round 0: 245,784 requests in  15.29s = 16,077.9 req/s  (40 chain steps)
round 1: 203,739 requests in  15.09s = 13,499.4 req/s  (40 chain steps)
round 2: 5,021,952 requests in 398.98s = 12,587.0 req/s (27 chain steps)
```

Round 2's chain step is the one that hit the open issue above (one of
its 27 wants never landed); the run ended there. The L2 throughput
numbers themselves (12.5k-16k req/s sustained, including through the
multi-minute confirmation wait inside round 2) are consistent with
earlier 46-consumer preprod numbers once host contention is accounted
for.

## L2 throughput: sustained firehose overlapping the L1 chain step

Each round, every live consumer's `firehose_l2` fires back to back for
the _entire_ duration of that round's L1 chain step (`want`/`flush`),
not just once before it starts -- `flush`'s confirmation wait is real
wall-clock time on preprod (seconds to tens of minutes); that whole
window fires L2 requests, not zero. `report_round`'s req/s is
throughput sustained through that wait, not a quick pre-chain-step
burst.

### Measured, real preprod, 46 consumers (`alice=46`, pre-refactor)

```
round 0: 1,714,565 requests in 177.98s = 9,633.4 req/s  (46 chain steps)
round 1: 1,096,962 requests in 117.52s = 9,334.0 req/s  (46 chain steps)
round 2:   711,486 requests in  78.54s = 9,058.5 req/s  (46 chain steps)
round 3:   597,780 requests in  66.67s = 8,966.8 req/s  (36 chain steps)
```

46 was picked to sit right at the real per-tx ceiling for consumer-
signed steps: the round 0 Add batch (46 wants) was rejected once
(`MaxTxSizeUTxO`: supplied 17994-18274, expected 16384) and retried at
39, landing. Matches the per-tx channel ceiling measured previously
(39-46 channels/tx, consumer-signed). (`Basket::MAX_OPENS_PER_TX` is
now a flat 40-channel cap on the `console`-based engine, chosen as a
simple, conservative starting point rather than a measured ceiling --
see its own doc comment.)

### Measured, real preprod, 4 consumers (default roster, pre-refactor)

```
round 0:   345,092 requests in  87.87s = 3,927.5 req/s  (4 chain steps)
round 1:   184,608 requests in  77.58s = 2,379.6 req/s  (4 chain steps)
round 2:   245,920 requests in  79.36s = 3,098.7 req/s  (4 chain steps)
round 3: 1,138,400 requests in 276.74s = 4,113.6 req/s  (4 chain steps)
```

Full lifecycle completed, `spent == committed` across all consumers.

### Reading these numbers

This host shares its real CPU across however many consumers are
firehosing and the server process under test (and this sandbox's
visible core count may not match its real CPU budget). The absolute
req/s above reflects host contention as much as `subbit-gate`'s real
capacity -- real numbers, not a rigorous isolated benchmark. Worth
collecting anyway.

## L1 throughput (measured, real preprod, pre-refactor)

Fees read back from Blockfrost per landed tx, 200-consumer `alice` run:

| Step                           | Channels/tx (after backoff) | Fee/channel             |
| ------------------------------ | --------------------------- | ----------------------- |
| Open (no per-consumer witness) | 41-80                       | ~11,100-13,800 lovelace |
| Add (consumer-signed)          | 39-46                       | ~38,700-47,100 lovelace |

Add/Sub costs roughly 3-4x Open per channel -- the extra per-consumer
witness, not the step logic itself. (See "Open issue" above: this gap
is also the leading suspect for why consumer-signed steps specifically
are the ones seen failing to land post-refactor.)

## `--resume <round>`

Round schedule is a process-global counter, not derived from on-chain
state. `--resume <round>` sets the starting round, for continuing a
run that crashed or hit a safety-net timeout.

- `check()` skips its "channel must still be Opened" refusal when
  `resume_round > 0`. Duplicate-tag check still runs regardless.
- A fully `Settled` (vanished) consumer and a never-opened one look
  identical to `open_pending`. Resuming long after most of a run
  already finished can reopen fresh channels for everyone who already
  completed. Check `--status` first if unsure.

## `--tidy`

Closes/Settles/Ends every recognized channel regardless of stage,
until none remain. `--scan-consumers` bounds how many consumer
_indices_ it checks -- set it at least as high as the largest roster
any earlier run under the same `L2_SEED_HEX` used.

## `MAX_LOOP_TIME` vs `close_upper`'s margin (structural, now fixed)

`close_upper`'s 600s margin (bumped from 120s earlier this session --
see its own doc comment in `orchestrator.rs`) pushes `elapse_at` out by
~660s (600s margin + 60s `close_period`) from whenever a `Close` round
_starts_, not from when it lands -- so no consumer can even want
`Elapse` until ~660s in, on top of however long the `Close` round
itself takes to land under load. `MAX_LOOP_TIME` (900s, the whole
run's abandon-timeout) was never raised to match when the margin was
bumped. Repro: a 40-consumer run hit the 900s wall 4 times in a row,
each time with round 2 (the `Close` round) alone taking 790-845s,
leaving no margin for round 3+ (`Elapse`) to ever complete. Fixed by
raising `MAX_LOOP_TIME` to 2400s.

## `--no-elapse`

`Orchestrator::new` normally marks one consumer (of >=2) to `tidy` once
`Closed`, deliberately skipping `Settle` to demonstrate the `Elapse`
path -- which is exactly the path gated by the 600s margin above.
`--no-elapse` skips that demo entirely, so every consumer settles via
the real spawned `subbit-claim` process instead -- reachable as soon as
`Close` lands, no margin wait -- for a run that only needs to prove the
lifecycle completes, not specifically exercise `Elapse`. `subbit-claim`
settles a `Closed` channel regardless of whether any funds are owed.

Verified live (`alice=3 richy=1 --no-elapse`, real preprod, post-
refactor): full lifecycle completed in 145 rounds with no `Elapse`
wait, `spent == committed` (28,798,700 total) across all 4 consumers.

```
round 0:  32,284 requests in  14.48s = 2,229.7 req/s (4 chain steps)
round 1:  53,586 requests in  30.51s = 1,756.3 req/s (4 chain steps)
round 2:  66,195 requests in  30.51s = 2,169.5 req/s (2 chain steps)
round 3: 125,902 requests in  60.58s = 2,078.4 req/s (2 chain steps)
round 4:  10,020 requests in   2.34s = 4,281.2 req/s (1 chain step)
```

In line with the pre-refactor 4-consumer baseline above
(2.4k-4.1k req/s) -- not comparable to the 40-46 consumer runs' 9k-16k
req/s, which scale with consumer/firehose count, not with this flag.

## L2 throughput investigation: gate + stamp + canary (this session)

- **Not CPU-bound.** Bracketed `/proc` CPU-tick sampling mid-burst
  (40-consumer `richy:burst`, real preprod): gate+canary combined used
  ~4.72 of 12 host cores. Ruled out CPU saturation as the limiter.
- **Echo baseline** (zero business logic, bare axum): 170-210k req/s is
  this stack/host's hard ceiling.
- **`firehose.rs` bench bug, fixed:** always resigned a fresh `Pop`
  (full ed25519 verify, ~105us/req under contention) instead of
  reusing a cached `Mac` (~2.9us/req). Fix raised firehose's own
  measured throughput ~57k -> 174,952 req/s, matching the echo ceiling.
- **Real `subbit_stamp::Stamp` had the same bug, different shape:**
  unconditionally `verify_iou`'d every response regardless of auth
  type. Fixed generally in `apply_status`: only verify/commit when the
  echoed amount exceeds what's already cached (an `Iou` only
  increases).
- **x402 dialect parity bug, fixed:** was missing the mac-invalidation
  reaction vanilla already had on an `InvalidAuth` error; added, plus
  a parity test.
- **New `Stamp::always_by_id()` mode:** every request a fresh signed
  `Iou` over `Request::ById`, no Pop/Mac liveness proof at all.
  Firehose's own `always_by_id` ceiling plateaus ~50-51k req/s (vs the
  mac-cached mode's ~174-210k) -- the honest floor for "every request
  independently fully authenticated," since there's no cheap liveness
  layer to amortize against.
- **gate `spawn_blocking` thread-pool oversubscription, fixed:**
  unbounded -> 200-250 threads at concurrency=160. Bounded via a
  semaphore sized to `available_parallelism()`; tail latency p99
  19-21ms -> 7.7-8.3ms, also fixed a slows-down-over-sustained-runs
  degradation.
- **db/lock layer ruled out:** exposure-lock wait ~47-220ns, flush
  ~690-730us amortized to ~1.2us/request -- both negligible.
- **Open: gate's commit-phase cost under canary's topology is higher
  than expected.** `spend_diag` measured `avg_commit_ns` ~349-358us
  during a real canary run, vs the ~33us raw / ~105us contended
  `verify_iou` cost measured in isolation. Not explained -- candidate
  causes: lock/channel-state contention under 40-way concurrent `ById`
  commits, or commit bookkeeping beyond the signature check itself.
- **Open: canary's own round-trip latency is a stable ~2.5x firehose's.**
  p50 ~1.8-1.9ms vs firehose's 724.7us at the same concurrency=40,
  reproduced across multiple runs, matching the throughput ratio
  (canary steady-state ~20-30k req/s vs firehose's ~51k). Not
  CPU-bound; not yet isolated to client vs. server vs. network.
- **Benchmark bug, fixed: round-0 `Close` race.** `richy:burst`'s
  `want()` offered `Close` the instant `round+1==rounds()` (round 0,
  immediately) -- whichever consumer's `Open` happened to confirm
  first also raced to close first, tiering live concurrency down over
  the run (40 -> 20 chain-steps) and contaminating throughput/latency
  readings with declining concurrency. Fixed: burst consumers now hold
  their channel open for a fixed `close_after` (45s) measured from
  their own construction time, not round index, so they all start
  competing for `Close` together.
- **Residual, unfixed: chain-level `Close`-batching capacity.** Even
  after the round-0 race is fixed, roughly half of 40 simultaneous
  `Close` proposals still take many extra rounds to confirm -- a real
  constraint inside `subbit_console::submit`'s batching/retry,
  downstream of anything the canary scenario controls. Not chased
  further this session.
- **Measurement-interference bug, fixed:** canary's own latency
  instrumentation originally locked a shared `Mutex<Vec<Duration>>` on
  every request (40-way contention at tens of thousands of ops/sec),
  depressing measured throughput ~30% (31.4k -> 22.0k req/s on an
  otherwise-identical run). Fixed by accumulating per-task-local and
  merging once at task end (the same pattern firehose already used);
  the percentiles themselves were unaffected (sampled before the
  lock), only the aggregate req/s was contaminated.
- Full lifecycle still completes cleanly end-to-end with every fix
  applied (`spent == committed` reconciled across all 40 consumers,
  multiple runs).
