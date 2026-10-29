# canary results

`packages/canary` drives a full subbit channel lifecycle (Open -> Add ->
Close -> Settle/Elapse -> End) against an in-memory mock chain or real
Cardano preprod via Floe, and doubles as the L2 throughput benchmark
for `subbit-gate`.

## Running it

```
canary --mode mock
canary --mode preprod --persona alice=46
canary --mode preprod --status --scan-consumers 60
canary --mode preprod --tidy --scan-consumers 60
canary --mode preprod --persona alice=46 --resume 4
canary --mode preprod --mimic --persona alice=1
```

`--mode preprod` needs `WALLET_SKEY`, `L2_SEED_HEX` in the environment
(`.env` at the repo root).

**Always run `--release`.** Debug-mode elliptic-curve signing measured
~50-60x slower than release in this codebase -- a debug run's numbers
have no relationship to real throughput. Canary itself warns loudly on
startup if built in debug mode.

## L2 throughput: sustained firehose overlapping the L1 chain step

Each round, every live consumer's `firehose_l2` fires back to back for
the _entire_ duration of that round's L1 chain step (`want`/`flush`/
`claim_tick`), not just once before it starts. `flush`'s confirmation
wait is real wall-clock time on preprod (seconds to tens of seconds);
previously that whole window fired zero L2 requests. Now
`report_round`'s req/s is throughput sustained through that wait, not
a quick pre-chain-step burst.

Implementation: `Orchestrator::run_until_done` spawns a loop (closed
over a cloned `Vec<Arc<Consumer>>`, stopped via a plain `Arc<AtomicBool>`)
that keeps calling `firehose_l2` on every consumer until the chain step
resolves, then folds its total into `spent`.

### Measured, real preprod, 46 consumers (`alice=46`)

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
(39-46 channels/tx, consumer-signed).

### Measured, real preprod, 4 consumers (default roster)

```
round 0:   345,092 requests in  87.87s = 3,927.5 req/s  (4 chain steps)
round 1:   184,608 requests in  77.58s = 2,379.6 req/s  (4 chain steps)
round 2:   245,920 requests in  79.36s = 3,098.7 req/s  (4 chain steps)
round 3: 1,138,400 requests in 276.74s = 4,113.6 req/s  (4 chain steps)
```

Full lifecycle completed, `spent == committed` across all consumers.

### Measured, mock, 4 consumers (default roster)

```
round 0: 28,526 requests in 10.11s = 2,821.6 req/s  (4 chain steps)
round 1: 30,624 requests in 10.09s = 3,033.7 req/s  (4 chain steps)
round 2: 34,970 requests in 10.09s = 3,466.8 req/s  (4 chain steps)
round 3: 22,050 requests in 10.06s = 2,192.6 req/s  (3 chain steps)
round 4: 19,624 requests in 10.04s = 1,955.0 req/s  (2 chain steps)
```

### Reading these numbers

This host shares its real CPU across however many consumers are
firehosing and the server process under test (and, per an earlier
session's own finding, this sandbox's visible core count may not
match its real CPU budget). The absolute req/s above reflects host
contention as much as `subbit-gate`'s real capacity -- real numbers,
not a rigorous isolated benchmark. Worth collecting anyway.

### Known failure mode: `Elapse` submitted too early

A 46-consumer preprod run hit a hard failure at round 4:

```
OutsideValidityIntervalUTxO (invalidBefore = SlotNo 136420459) (SlotNo 136420455)
```

A single `Elapse` want's validity-interval lower bound (`Consumer::
flee_want`'s `after + 1s`) landed 4 slots ahead of the chain's own
slot at submission time. `L1::flush` drains successful sub-batches as
it backs off, so 18 of that round's 19 wants had already landed by
the time the 19th (the bad `Elapse`) hit a singleton batch and
aborted the whole call with a fatal error, ending the process.

Confirmed via `--tidy --scan-consumers 60` immediately after: exactly
one channel remained (the one deliberately-fleeing straggler every
run marks to demonstrate `Elapse` instead of `subbit-claim`
settlement) -- every other consumer had already been settled by
`subbit-claim` in earlier successful rounds. `--tidy` reclaimed it;
clean final state, nothing left on-chain.

**Update (`--mimic` session, see below):** `+1s` was bumped to `+20s`,
then live-reproduced _still_ failing by 33-53 slots short even at
`+20s` -- not clock skew (`crate::time::now()` matches an external
HTTP `Date` header exactly) and not a stale `posix_to_slot` mapping
(its constants reproduce Koios's own ground-truth slot for the
current wall-clock time exactly). Most likely amaru's own mempool
"current slot" view trailing wall-clock-projected slot during a
block-production gap on preprod. Fixed two ways: the margin is now
`+120s` (matching `close_upper`'s own established margin, same
file), and `L1::flush` now parses this exact rejection shape
("current slot `N` not within transaction validity interval
`[M, +∞)`") and waits out the reported shortfall (+5s) before
retrying the same batch, up to 10 times, rather than aborting.

## L1 throughput (measured, real preprod)

Fees read back from Blockfrost per landed tx, 200-consumer `alice` run:

| Step                           | Channels/tx (after backoff) | Fee/channel             |
| ------------------------------ | --------------------------- | ----------------------- |
| Open (no per-consumer witness) | 41-80                       | ~11,100-13,800 lovelace |
| Add (consumer-signed)          | 39-46                       | ~38,700-47,100 lovelace |

Add/Sub costs roughly 3-4x Open per channel -- the extra per-consumer
witness, not the step logic itself.

## Confirmation waits: growing backoff

`L1::wait_for_confirmation` retries the preprod poll window (10s x 30,
~5min) with growing backoff on timeout: 30s/60s/120s capped, up to 4
windows (~20min worst case). A timed-out window isn't proof of
failure -- indexing can lag. Check `/txs/{hash}` on Blockfrost directly
(404 = never confirmed) if in doubt.

## `--resume <round>`

Round schedule is a process-global counter, not derived from on-chain
state. `--resume <round>` sets the starting round, for continuing a
run that crashed or hit the 900s `MAX_LOOP_TIME` safety net.

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
any earlier run under the same `L2_SEED_HEX` used. Always also
attempts to reclaim a `--mimic` decoy for each consumer index, win or
no-op, whether or not `--mimic` is passed -- tidy's whole point is
reclaiming leftover state regardless of what produced it.

A long retry storm (repeated `MaxTxSizeUTxO`/`OutOfExError` shrink
cycles) can burn enough time that a batch's validity upper bound
expires before landing, failing with `OutsideValidityIntervalUTxO`
regardless of batch size. Retry `--tidy` (idempotent, recomputes a
fresh bound each call).

**Update (`--mimic` session):** tidy used to refuse outright
("refusing to guess which one") if more than one live channel shared
one identity -- exactly the shape a `--mimic` run interrupted
mid-way (crash, kill, timeout) leaves behind, since nothing re-checks
"does a decoy already exist" the way `open_pending` does for real
consumers. Per the explicit requirement that tidy must always
succeed given the right signing keys, `run_until_done`'s own
per-identity lookup (which tidy drives) is now tolerant of more than
one match (`L1::channels_for_consumer`, driving every match
independently) instead of erroring; the strict, ambiguity-refusing
lookup (`can_for_consumer`) stays for `check()`'s own pre-flight
safety net on a _fresh_ run, where more than one match really is a
bug worth stopping for. Verified live: both of this session's
abandoned seeds (each left with a genuine duplicate decoy from an
earlier interrupted attempt) tidied clean in one `--tidy` call each
(one needed a retry purely for the `Elapse`-timing reason above, not
the duplicate itself).

## `--mimic`: does a decoy sharing a real channel's keytag cause bad behavior?

`canary --mode preprod --mimic --persona ...` runs an otherwise
normal scenario with one extra decoy channel per real consumer,
opened sharing that consumer's own `(iou_key, tag)` under a
different `consumer` identity, closed immediately (while the real
channel is still freshly `Opened`) -- then the _real_ `subbit-claim`
and `lineage-index` binaries are spawned as child processes (own
isolated, freshly-funded fee wallet) in place of canary's in-process
`claim_tick`/`index_via_floe` shortcuts, so the actual claim/index
pipeline is what has to get this right, not canary's own shortcuts.
The test is simply whether the normal scenario still completes
correctly with decoys thrown in -- no bespoke mid-scenario assertion
needed, since `decide.rs` already gates `Settle` on each input's own
on-chain stage (confirmed by inspection: a shared keytag structurally
can't make it settle something still `Opened`).

**Confirmed live** (full run, `alice=1`, exit 0): the real channel
completed its whole lifecycle (Open -> Add x3 -> Close -> Settle ->
End) and the decoy was independently Settled _and_ Ended -- in one
case, `subbit-claim` batched the real channel's own Settle and the
decoy's Settle into a single transaction, each against its own
provably-owed share of the shared IOU:

- Batched Settle (both the real channel and the decoy, one tx):
  [`dc8206f9cd84e3732f39a3decbd5137b89d51049da8f89ab4cc5ff2eeb856919`](https://preprod.cexplorer.io/tx/dc8206f9cd84e3732f39a3decbd5137b89d51049da8f89ab4cc5ff2eeb856919)
- Real channel's final `End` (full clean run):
  [`7f018ac67e4af5ad1ea71bc6a3ebb5946719936fa1f89a08c6d826ec26cf555a`](https://preprod.cexplorer.io/tx/7f018ac67e4af5ad1ea71bc6a3ebb5946719936fa1f89a08c6d826ec26cf555a)
- Decoy's own final `End` (same run):
  [`ad0b43dfa78cf6fedfbbe5323a0f0e972375c1a6dcb443d5c4575ca61c4593c2`](https://preprod.cexplorer.io/tx/ad0b43dfa78cf6fedfbbe5323a0f0e972375c1a6dcb443d5c4575ca61c4593c2)

`Stagish` (the lineage DTO) has no `Settled` variant --
`subbit_tx::variables::settle` always transitions to on-chain
`Stage::Settled` with a continuing output holding the remainder,
never a bare removal -- so a decoy needs its _own_ follow-up
`Want::End` once settled, same as any real consumer. First
implemented as a bespoke `Orchestrator::settle_mimics` loop
reimplementing that decision by hand; **that was the wrong shape**
-- `Consumer::want`'s very first check is already `can.contains(&Can
::End)`, unconditionally, real or fleeing. Fixed by making each
decoy an actual `.flee()`-ing `Consumer` (built from the decoy's own
seed, with `iou_key`/`tag`/`deposit` overwritten to match the real
consumer it's posing alongside -- the same trick `--tidy`'s own
mimic cleanup already used), pushed into the ordinary consumer list
`open_pending`/`run_until_done` already drive. `settle_mimics` and
`open_and_close_mimics` are gone entirely; a decoy's `Close`/
`Elapse`/`End` now all come from the exact same generic tx-builder
path a real consumer's does, no parallel implementation to keep in
sync.

Bugs found and fixed along the way (all in canary's own harness code
or its newly-added `FloeConnector`, not in `subbit-claim`/
`subbit-index` themselves, which performed correctly throughout once
the harness could observe them properly):

- `subbit_claim::floe::to_output` dropped reference-script info
  unconditionally -- fine for claim's own use, but once
  `FloeConnector::utxos_at` (canary) reused it generically for every
  tracked address, it broke `cardano_session::Session::ref_script`'s
  generic script discovery: a live ref-script upload landed and
  indexed correctly, but was never recognized. Fixed: round-trip
  `script` faithfully (`PlutusVersion`'s discriminant matches
  `ScriptLang`'s 1:1).
- `upload_validator_if_missing` never waited for its own upload tx to
  confirm before trusting it had landed -- fixed, via the existing
  `wait_til`.
- `ProtocolParameters::preprod()`'s `fee_per_byte`/`fee_constant`
  (44/155381) match live preprod's real `min_fee_a`/`min_fee_b`
  exactly (checked against Koios) -- yet a plain ref-script upload
  was rejected as underfeed by ~15%. Root cause not fully chased down
  (not the two obvious culprits); worked around with a flat 150%
  safety-margin multiplier in `FloeConnector::protocol_parameters`
  (canary only -- a real production service like `subbit-claim`
  should not copy this).
- `run_until_done`'s loop only ever refreshed its cached channel view
  as a side effect of _its own_ submissions -- broke the moment
  claiming moved to an externally-spawned `subbit-claim` process
  (nothing canary itself submits ever changes the final `Settle`).
  Fixed: unconditional `self.l1.refresh()` once per loop iteration.
- Canary's own loop and the real, concurrently-spawned `subbit-claim`
  process (`--mimic`) both touch the same real consumer's channel,
  uncoordinated -- latent the moment claiming moved to a separate
  process, surfaced live once a `too_early_wait` retry (above) gave
  `subbit-claim`'s 5s poll a ~40s window to `Sub` a channel out from
  under a stale-built `Close`/`Add`, failing with "unknown (but
  required) transaction input". Fixed two ways: `L1::flush` refreshes
  before every retry (not just the first attempt), and when amaru
  names a specific already-spent input that matches one of this
  batch's own wills, that single want is dropped (not a failure --
  the other process already moved that channel on) and the rest
  retried, rather than failing the whole batch over it.

**Final clean run** (`alice=1`, x402, exit 0, confirms every fix
above together): both the real channel and the decoy went all the
way through, entirely via the generic per-consumer path -- no
separate mimic bookkeeping printed anywhere in the log, and the L2
summary correctly counted 1 consumer, not 2 (`print_l2_summary` now
filters out personaless decoys). Open tx (both channels batched
together):
[`35f12700a851fb2b0f821281a4d22e6004a46a03f24b211479bf4d46988f9499`](https://preprod.cexplorer.io/tx/35f12700a851fb2b0f821281a4d22e6004a46a03f24b211479bf4d46988f9499).
