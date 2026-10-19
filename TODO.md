# TODO

Items with no other home.

- [ ] `accepted_matches` (`packages/server/src/dialect/x402.rs`) only matches the canonical
      CAIP-2 form (`Network::to_caip2`). The real x402 Cardano `exact` scheme spec requires a
      facilitator to also accept the CIP-34 numeric alias (`Network::to_cip34`) as equivalent when
      matching an echoed `accepted.network`.
- [ ] `examples/echo/proxy` need full upgrade: demo with different cost models and dialects.
- [ ] Test: an end-to-end `subbit-e2e` test driving `ByEgress`/`ByClaim` partial refund over
      real HTTP (spend, serve, refund, check the resulting header), for both dialects. Unit-level
      dialect adapter tests already cover this; no real HTTP coverage yet.

- [ ] Support mutual: implement a real `MutualService` (parse the consumer's close transaction,
      verify none of its inputs belong to the provider, verify the channel's keytag is an input and
      the first output pays the provider correctly, co-sign, broadcast). See `Mutual`'s own doc
      comment in `packages/l2-dto/src/mutual.rs`.
- [ ] Test: mutual support. Adapter-level tests once a real `MutualService` exists, plus growing
      `packages/conformance` to cover the mutual-spend path (`iou`/`wire` only today).

- [ ] `subbit-client`/`examples/echo/proxy` body plumbing: carry a body through `Outcome`,
      written back verbatim on a 402, so an x402 client can complete a real round trip through the
      proxy (today `Outcome` only carries a status code and an optional header).
- [ ] Test: an x402 round trip through `examples/echo/proxy` itself, not just in-process against
      `Ctx`.

- [ ] Vanilla's `.well-known/subbit` discovery: route the existing `Terms` wire type (the
      type exists, nothing serves it).
- [ ] Test: the discovery endpoint, once routed.

- [ ] Adapter tests against a real captured x402 shape. Both server's and the issuer's dialect
      tests currently only check our own round-trip, not a captured real x402 client's expectations
      (the habit `real_world_402_response_from_the_reference_spec` already has one layer down).

- [ ] Decide validator-conformance division of labor: Aiken-native `test` blocks (properties the
      validator can state about itself) vs `packages/conformance`'s UPLC-eval approach ("does Rust's
      encoding agree with the validator").

- [ ] Remaining zero-test crates: `subbit-cli`, `subbit-client`, `subbit-session`,
      `subbit-config`.

- [ ] Preprod harness: a funded test wallet (public all-`abandon` mnemonic), a preprod
      Blockfrost project id or Koios, real deposit/claim/refund transactions actually submitted, the
      real `naive` indexer watching for them, a CI job on `workflow_dispatch`/a schedule, never
      gating a PR.
- [ ] Test: preprod harness scenarios, once it exists. The tier-3 lifecycle for real, a real
      cooperative close once mutual support lands, the same flow through
      `examples/echo/proxy` once its body-plumbing gap closes.

## Decided, not a gap

Vanilla's `refund` stays unauthenticated. It is only ever reached as a same-request
continuation of a spend that already authenticated the caller, not a standalone call a consumer
can reach cold. Adding a fresh liveness check to it would be redundant, not a fix.
