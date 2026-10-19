# canary results

`packages/canary` drives a full subbit channel lifecycle (Open -> Add ->
Close -> Settle/Elapse -> End) against an in-memory mock chain or real
Cardano preprod via Blockfrost, and doubles as the L2 throughput
benchmark for `subbit-server`.

## Running it

```
canary --mode mock
canary --mode preprod --persona alice=46
canary --mode preprod --status --scan-consumers 60
canary --mode preprod --tidy --scan-consumers 60
canary --mode preprod --persona alice=46 --resume 4
```

`--mode preprod` needs `BLOCKFROST_PROJECT_ID`, `CANARY_SIGNING_KEY_HEX`,
`L2_SEED_HEX` in the environment (`.env` at the repo root).

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
contention as much as `subbit-server`'s real capacity -- real numbers,
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
aborted the whole call with a fatal error, ending the process. `+1s`
is too tight a margin against real preprod slot timing. Not fixed
yet; widening that margin is the likely fix.

Confirmed via `--tidy --scan-consumers 60` immediately after: exactly
one channel remained (the one deliberately-fleeing straggler every
run marks to demonstrate `Elapse` instead of `subbit-claim`
settlement) -- every other consumer had already been settled by
`subbit-claim` in earlier successful rounds. `--tidy` reclaimed it;
clean final state, nothing left on-chain.

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
any earlier run under the same `L2_SEED_HEX` used.

A long retry storm (repeated `MaxTxSizeUTxO`/`OutOfExError` shrink
cycles) can burn enough time that a batch's validity upper bound
expires before landing, failing with `OutsideValidityIntervalUTxO`
regardless of batch size. Retry `--tidy` (idempotent, recomputes a
fresh bound each call).
