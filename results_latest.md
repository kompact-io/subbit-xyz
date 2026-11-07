# canary: latest result (2026-11-07)

Scenario: `alice:burst=40 --flat-cost 14 --burst-target-lovelace 1000000 --no-elapse`, release
build, real Cardano preprod via Floe. Exposure config (`packages/canary/src/provider.rs`):
`buffer.max_pending`=10 ADA, `exposure.single_write_threshold`=1 ADA,
`exposure.max_cumulative_exposure`=10 ADA, both flush/checkpoint intervals=1s.

## Result

- Full lifecycle completed cleanly: `spent == committed` exactly, 40,000,240 lovelace total
  across 40 consumers (16 Vanilla, 24 X402).
- L2 throughput: 2,857,160 spend requests in 106.63s = **26,794.5 req/s** (concurrency=40).
- L2 latency: p50=1.448ms p90=2.177ms p99=2.988ms.
- Reproduced twice more under the identical scenario: 23,476.4 req/s and 27,793.3 req/s -- all
  three cluster tightly (23k-28k), consistent with the Little's-Law cross-check
  (concurrency / per-request-latency ≈ 28,000 req/s).

## Transactions (preprod)

| Step  | Channels | Confirm time | Tx                                                                                                                         |
| ----- | -------- | ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Open  | 40       | ~60.3s       | [`2ef0638e…`](https://preprod.cardanoscan.io/transaction/2ef0638e1385004230a203fbb1fbc15f1f975b6b72ef7acd6f9aab9fba591e74) |
| Close | 40       | ~14.2s       | [`d2cd3c29…`](https://preprod.cardanoscan.io/transaction/d2cd3c295ab622dc9b59dc339b0a246106f02a2ee1c1560248e2389a6bc82287) |
| End   | 40       | ~120.5s      | [`1b5afa4e…`](https://preprod.cardanoscan.io/transaction/1b5afa4ebb4d7f416377ed8e2ad6efa35137c799d1a2bfbfa0ae7854475d70e4) |

Plus one incidental tx right after Open (reference-script deployment, ahead of the batched
Close/End above): [`a170a460…`](https://preprod.cardanoscan.io/transaction/a170a460878e6c261703688efe77a5fa712b8c56af84c7323659d56036c57346).

Settlement (the `Closed -> Settled` step, between Close and End) runs asynchronously in the
spawned `subbit-claim` process and isn't captured in this log's tx list.

## Bugs & issues

- **Fixed:** burst-mode round throughput was computed from the L1 chain step's own timer, not
  the L2 firehose's -- reported 186k req/s once, ~6.6x off from what the run's own latency
  implied. Fixed in `orchestrator.rs` by timing the firehose task independently
  (`spend_elapsed`), reported alongside (not in place of) `chain_elapsed`. Full writeup in
  `results.md`.
- No other issues this run -- Open/Close/End each landed on the first attempt, no retries, no
  rejections.
