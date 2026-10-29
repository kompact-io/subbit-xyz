# Plan: subbit-console — consumer's web app for managing subbits

## Context

`subbit-console` assumes `subbit-cli`'s
existing interactive `tx` flow (`packages/cli/src/tx.rs`, `tx/forms.rs`,
`tx/select.rs`, built on `inquire` prompts: stage → open/propose → submit) —
with a browser app.

Decisions already made (via two rounds of Q&A):

- **Scope**: full consumer-side lifecycle — Open, Add, Close, Elapse, End.
  `Sub`/`Settle` stay provider-only (`packages/claim`'s territory), out of
  scope here.
- **L1 chain data**: Floe (`kompact-io/floe`, a sibling repo this team
  owns) — via `floe-client`, this crate's own `wasm` feature forwarding
  its `http-json` on top (see section C — superseded by the `floe`
  dependency bump: no `Backend`-over-`fetch` impl of our own any more,
  `floe_client::http::json::Client` is used directly). `submit_tx` is
  unimplemented there (`bail!(...)`) — irrelevant, submission goes
  through CIP-30 instead.
- **Floe endpoint**: a plain user-supplied config value (like a base URL).
  Whether/how an operator exposes Floe over CORS/publicly is explicitly
  **out of scope** for this plan — Console just assumes whatever endpoint
  it's given works.
- **Wallet**: CIP-30, as already established — address/vkh, `getUtxos()`
  for fee inputs, `signTx`/`submitTx` for the actual signing+submission.
- **Terms discovery**: `.well-known/subbit` isn't routed server-side today
  because subbit expects the gate to sit behind a proxy anyway (per
  TODO.md and the user). Console doesn't depend on that TODO item landing —
  given a provider base_url, it tries sensible well-known path(s) itself.
- **Networks**: preprod **and** mainnet from the start.
- **close_period**, **provider**: fetched `Terms` pre-fills the form; the
  consumer can still edit afterward, flagged ("potentially not recognized
  by provider") if it no longer matches what was fetched — see section F.
  **asset** is the one exception: locked, not editable, whenever a `Terms`
  is attached — see section J.
- **IOU key/account custody**: support **both** directions — Console can
  generate a fresh account (signing key + tag) itself, or accept an
  already-existing public key + tag pasted in. See section E.
  Input accepts signing or verifying key (only verifying key required, but signing key
  needs to be exported if generated).
- **Stamp integration scope for MVP**: data-only. Console produces/accepts
  the same `{signing_key, tag}` (+ a TTL) shape `subbit-stamp` already uses
  as its `Config` (`packages/stamp/src/stamp.rs::Config` /
  `account::Config`) — it does not embed or run Stamp's own wasm logic.
- **Dashboard-first**: Console must render a useful view (discovered
  channels) with **no keyring and no wallet connected at all** — plain,
  unfiltered discovery via Floe. A keyring and/or a connected wallet, when
  present, narrow that same view (preferred filter/highlight), they are not
  a precondition for seeing anything. Connecting a wallet is required only
  to _enable tx-building buttons_ (Open/Add/Close/Elapse/End), never to
  browse. See section J.
- **Keyring**: Console carries its own keyring, parallel to — not shared
  with, not upstreamed from — `packages/claim`'s `keyring.rs`/`cli`'s
  `cmd/keyring.rs` pattern (labelled keys, each key's stamp-compatible
  32-byte ed25519 seed). Keys must support a **verifying-only** form (no
  signing key held) as a first-class case, not a degraded one — see
  section E.
- **Providers**: a labelled list of known providers (base_url + cached
  `Terms`), re-fetchable on demand, and manually editable/addable for
  providers with no live `.well-known` endpoint yet — see section F.
- **Persistence**: required, not deferred. Native: a file (mirroring
  `packages/cli/src/config.rs`'s `Config::load`/`save`). Wasm: browser
  `localStorage`. Console must expose explicit data-management actions
  (Export, Import, Delete — with confirmation) rather than leaving the
  user's keyring/providers silently at the mercy of browser storage —
  see section G.
- **Error handling**: one top-level `Error` (`thiserror`) for Console's own
  new Rust logic, mirroring the per-module pattern already used by
  `claim::keyring::Error`/`stamp::stamp::ResponseError`. Wasm must never
  panic — see section H.
- **Open-channel validation**: asset class is _locked_ to `Terms.asset`
  when a `Terms` is attached (not just warned about) — no way to submit a
  mismatched asset. A non-blocking warning (permitted to proceed) is shown
  when the connected wallet's balance of that asset is below `Terms.
min_iou_amount`, and separately when a chosen tag collides with one
  already known for the same iou_key (legitimate reuse — see docs/adrs/
  x402.md's keytag note). See section J.

## Proposal

### A. `subbit-tx` — Parse/Build logic, reused by `subbit-console` directly

**Revised: `subbit-tx` itself never gets a `wasm` feature.** The original
plan had `packages/tx/src/wasm.rs` as its own `#[wasm_bindgen]` surface
(`JsChannel`/`JsTx`), mirroring `subbit-stamp`'s `wasm` feature pattern.
That was tried, verified working (`cargo check`/`clippy` clean on
`wasm32-unknown-unknown`, a real `nix build` producing a working pkg),
and then _removed_ once it became clear it was the wrong shape: "what we
want is a unified core with thin wrappers to make it Native or Wasm" —
`subbit-tx` growing its own, browser-only wasm surface meant a future
native wrapper (a TUI, "an interface very similar to the web-app") would
have had nothing to reuse and would need to reimplement the exact same
basket logic from scratch. See section I for where this logic actually
lives now (`subbit-console`'s `Basket`, section E), with `subbit-tx`
staying entirely `wasm_bindgen`-free — a pure logic crate, same as it
always was for native use, now also directly depended on by
`subbit-console`.

What `subbit-tx` _does_ still carry, because both the native and wasm
paths need it:

- **Parse**: `Channel::try_from(&Output)` (`channel.rs`) + `Channel::
can()` (`step.rs`'s `Can::{Add,Sub,Close,Settle,End,Elapse}`) — already
  existed, untouched.
- **Build**: `subbit_tx::tx::Tx` (`tx.rs`) end to end — `Tx::new`/
  `filtered`, `propose(input, Want)` (`Want::{Add,Close,End,Elapse}` from
  `step.rs` — `Sub`/`Settle` left unexposed), `add_open(Open)`, `build(
network_parameters, reference_utxo, change_address, fuel)` → a signed-
  ready `Transaction`. Unchanged; just wrapped by `Basket` now instead of
  by a `wasm.rs` of its own.
- **Network parameters, no fetch, no JS round-trip**: `NetworkParameters::
for_network(network: &str)` — now a public method on `subbit_tx::
NetworkParameters` itself (moved out of the old `wasm.rs`'s private
  helper, since `Basket::build` in `subbit-console` needs it too), still
  building `NetworkId::from(network)` + `ProtocolParameters::from
(network)` offline (mirroring `subbit_claim::config::Config::
network_parameters`'s own approach) and never crossing into JS as JSON.

Decision: Hard code network parameters per network.
We do not have a reliable way to source them yet.

### B. Shared Floe ⇄ cardano_sdk conversion, extracted for reuse

EDIT :: DO NOT DO THIS

`packages/claim/src/floe.rs` already has exactly the conversions Console
needs — `channel_from_utxo` (Floe `Utxo` → `subbit_tx::Channel`, for
display) and `to_output`/`to_input_output` (full-fidelity Floe `TxOutput` →
`cardano_sdk::Output`, for actually building a tx) — just currently private
to `claim`, a native-only crate.

Some of this will be duplicated in `subbit-console`.
We will not upstream these to a shared dependency. The footprint is small.

### C. `floe-client` — embedded directly, not a second wasm bundle

**Superseded.** Every earlier draft of this section assumed Console
had to reach Floe via _floe-client's own_ `wasm` feature — a
second, separately-built wasm bundle, because that feature's internal
workspace pins a `wasm-bindgen` version conflicting with ours. That
premise was wrong: the conflict is specific to `floe-client/wasm`
itself, not to depending on `floe-client` at all. Checked directly
against `floe-client`'s own `Cargo.toml`: every `wasm-bindgen`/
`js-sys`/`web-sys`/`wasm-bindgen-futures` dependency is `optional =
true` and reachable _only_ via the explicit `wasm` feature (`dep:`
syntax, no implicit back-door feature) — absent from `default`, and
`subbit-index` already proves a bare `default-features = false`
dependency on `floe-client` is wasm32-clean (its `cardano::
search_all`/`search_history`, generic over `floe_client::Backend`,
were already part of Console's own wasm build transitively).

So: `subbit-console` now depends on bare `floe-client` directly
(`default-features = false`, `wasm` never enabled), and supplies its
own `floe_client::Backend` impl over browser `fetch`
(`src/wasm/transport.rs`) under _our_ pinned `wasm-bindgen`/`web-sys`
— a ported, not copied-wholesale, version of floe-client's own
`wasm/transport.rs` (same `Backend` trait, same DTOs, reused as-is;
only the fetch plumbing itself, inherently wasm-bindgen-version-
specific, is duplicated). `src/floe.rs` (new core module) adds
`discover_live`/`discover_history`, generic over `impl Backend`,
reusing `subbit_index::cardano::search_all`/`search_history`'s
pagination directly — not a port — merging results across every
whitelisted validator (section K). `JsConsole`'s wasm surface gets two
free async fns, `fetchLiveChannels`/`fetchHistory` (free functions,
not `JsConsole` methods — wasm-bindgen can't safely hold `&mut self`
across an awaited `fetch`), feeding `basketRefreshChannelsJson`/
`reconstructLinesJson` respectively.

Net effect: **one wasm bundle, one `Cargo.lock`, no second flake
input.** `nix/console-www.nix` needs no change beyond its own stale
comment. Verified: `cargo check -p subbit-console --features wasm
--target wasm32-unknown-unknown` clean, `cargo test -p subbit-console`
(37 tests, incl. `floe::tests`' mock-`Backend` coverage of
`discover_live`/`discover_history`), `nix build .#subbit-console-wasm`
and `.#console-www` both clean end to end.

**Superseded again, by the `floe` dependency bump (workspace
`Cargo.toml`'s `amaru-*`/`wasm-bindgen`/`rust-toolchain.toml` update to
nightly-2026-09-04, floe-client rev `96df02e`).** `src/wasm/
transport.rs` is now deleted — the premise just above ("every
wasm-bindgen/js-sys/web-sys/wasm-bindgen-futures dependency is
`optional = true`... reachable only via the explicit `wasm` feature")
had already gone stale by the time this was checked again: current
floe-client no longer has a `wasm` feature at all, and
`gloo-net`/`js-sys`/`wasm-bindgen`/`wasm-bindgen-futures` are declared
as _unconditional_ `[target.'cfg(target_arch = "wasm32")'.
dependencies]` — present the moment anything depends on floe-client
for that target, feature flags or not. Our own workspace `wasm-bindgen`
pin is also now `=0.2.129`, the exact same pin floe-client's own
`Cargo.toml` uses — confirmed in `Cargo.lock`, one `wasm-bindgen` entry,
not two. So the conflict this whole section was built around is gone
on both counts that used to justify it: there's no way left to avoid
pulling floe-client's wasm32 deps in by toggling features, and even a
real second edge wouldn't conflict any more if there were one.

`src/floe.rs`'s `discover_live`/`discover_history` are unchanged (still
generic over `impl Backend`), but the concrete `Backend` now used by
`wasm/mod.rs`'s `fetchLiveTxos`/`fetchHistory` is floe-client's own
`floe_client::http::json::Client::connect(base)` — the same `Client`
floe-client's native (`reqwest`) and wasm32 (`gloo-net`) builds both
already provide, target-gated inside floe-client itself. Forwarded in
via this crate's own `wasm` feature (`floe-client/http-json`), not
turned on for `cli`/native. `wasm-bindgen-futures` stays a direct
dependency (needed by `#[wasm_bindgen]`'s own macro expansion over
`async fn`, not called by name from our code) but `web-sys`/`js-sys`
are gone entirely — nothing in this crate used them once the transport
impl did. `decode_response` (this section's own `src/floe.rs` helper,
reused only by the now-deleted transport) is deleted with it, test
included. Verified: `cargo check`/`clippy -D warnings -p subbit-console`
clean on both native and `--target wasm32-unknown-unknown`, with and
without `--features wasm`; `cargo test -p subbit-console` 39 passed
(40 minus `decode_response`'s own retired test), same with `--features
wasm`.

### D. Terms discovery

`Terms` (`packages/l2-dto/src/terms.rs`) already carries everything needed
to open a channel: `network`, `script` (validator hash, sanity-checked
against the Console build's own `VALIDATOR.hash`), `asset`, `provider`
(vkh), `close_period`, `confirmation_depth`, `min_iou_amount`, and a
resource `catalog` (useful later for the Stamp export, section E).

Given a user-supplied provider base_url, Console's JS tries the
established `.well-known/subbit` convention path (and can fall back to a
short list of other sensible locations if that 404s) via plain `fetch` —
no wasm needed for this step, it's a GET + JSON/CBOR parse. Decode with
`Terms`'s existing `serde`-derived shape.

### E. Keyring — and per-channel account/key custody

Rename note (already settled): the per-channel `{signing_key or vk, tag}`
JS-facing object is called a **Channel** (not "Account") — "channel" is
already context-specific in this codebase (L1 UTxO state vs. L2 keytag
identity), so this is one more sense of an already-overloaded word, not a
new ambiguity. This doc still says "L1 Channel" (section A's `Channel::
try_from`) vs. "L2 Channel" (this section) where it matters.

**Keyring** (new, console-only, parallel to but not sharing code with
`packages/claim/src/keyring.rs`): a labelled store of signing keys —
`BTreeMap<Label, [u8; 32]>` of stamp-compatible raw ed25519 seeds, same
shape as `claim`'s `KeyHex`/`cli`'s `cmd/keyring.rs`. One keyring key can
back multiple L2 Channels under different tags (already-anticipated reuse
pattern, `docs/adrs/x402.md`'s keytag note). Each entry also carries two
independent role flags, `is_consumer`/`is_iou`, set when the key is
added/generated: the _same_ ed25519 keypair can legitimately serve as
`Constants.consumer` (hashed to a vkh) and/or `Constants.iou_key` (used
as-is) for different channels, since both roles are the same underlying
primitive, just used two different ways.

**Verifying-only identities**: two _separate_, role-typed containers —
`Consumers` (`BTreeMap<Label, Hash28>`, matching `Constants.consumer`'s
own type) and `IouKeys` (`BTreeMap<Label, VerifyingKey>`, matching
`Constants.iou_key`'s own type) — not one generic container, because the
two roles need two different _types_, not just two different meanings
(an earlier draft of this plan had one `VerifyingKeyring` holding
`VerifyingKey` only, which could never actually have stored a wallet's
own vkh — a `Hash28` — without a type error; this split fixes that
latent bug structurally, not just by convention). Both mirror `Providers`'
label-based CRUD, but are additionally:

- **always populated live from the Keyring**: every `Keyring` entry
  flagged `is_consumer`/`is_iou` appears in the respective container's
  `effective()` view automatically, under the same label — the user
  never has to "export the vk" of a key they already hold, and nothing
  needs re-syncing if the Keyring changes (no stored duplicate, just a
  computed merge at read time).
- **independently extendable**: a verifying-only entry can also be added
  directly, with no corresponding signing key ever held — e.g. a pasted
  provider `vkh` into `Consumers`, or a key a wallet merely _advertises_
  (resolved below).
- **strictly invariant, both directions**: no duplicate label _or_ key
  value, checked eagerly against _both_ sources at once — `Keyring::
insert`/`generate` reject a flagged role whose derived identity
  already exists as an independent `Consumers`/`IouKeys` entry, and
  `Consumers::insert`/`IouKeys::insert` reject a label or value that
  already exists as a flagged `Keyring` entry. Enforced by each
  container's own methods (which require a `&Keyring` argument to do
  this check), never bypassable by reaching into a container's internal
  map directly — there isn't one to reach into, it's private.

All three containers persist via section G.

**Building an L2 Channel** (used in the Open-channel form, section J):
mirrors `subbit_stamp::Config { account: { signing_key, tag },
ttl_relative }` (`packages/stamp/src/stamp.rs`, `account.rs`) as the
interchange shape — same field names, so it matches a native `subbit-stamp`
TOML config or a future stamp-web (`JsStamp::new(signing_key, tag,
ttl_relative_secs)`, already in `packages/stamp/src/wasm.rs`).

- **Pick from Keyring**: select an existing labelled key, choose/generate a
  tag (warn-but-permit on collision, see decisions above).
- **Generate new**: mint a fresh ed25519 keypair (wasm32's `getrandom`
  backend — flagged as low-risk-but-unverified in the original tx-builder
  plan; worth a real smoke test here, not `cargo check` alone) + a tag;
  the new key is saved into the Keyring under a user-given label
  (persistence, section G) as part of the same action, not a separate
  step.
- **Export to Stamp is insisted on, not optional**: whichever path was
  used, Console always offers the resulting `{signing_key, tag,
ttl_relative}` (default `ttl_relative` e.g. 3600s, matching stamp's own
  placeholder default) as a downloadable/copyable JSON blob for the user's
  `subbit-stamp` instance — generating or picking a key inside Console is
  not useful unless it leaves the app in a form Stamp can consume.

**Resolved — the wallet's advertised key is a vkh, for `constants.consumer`
only, never `iou_key`**: CIP-30 doesn't export a raw signing key (`signTx`
only signs a _transaction_), but it does advertise the wallet's own vkh via
its address (`getUsedAddresses`/`getRewardAddresses`). That vkh is exactly
`constants.consumer` — the L1 identity of whoever's opening/stepping the
channel — and was already implicit any time a wallet is connected, nothing
new to build there. It is **not** a usable `iou_key` source: the L2 Channel
always needs a real, Console-held (or stamp-held) ed25519 signing key,
from the Keyring, because Stamp has to sign arbitrary protocol bytes
offline later and CIP-30 has no primitive for that (`signData`/CIP-8 signs
arbitrary messages too, but into a COSE_Sign1 envelope, not the raw
ed25519 signature `subbit_stamp::Account::sign`/`Iou` expects — not a
drop-in substitute). So: connecting a wallet contributes its vkh into
`Consumers` automatically — a `Hash28`, so it's now a type-correct fit
for that container specifically (see above) — session-only by default
(not persisted just from connecting), with an explicit "save to
Consumers" action if the user wants it labelled and persisted (section
G) for future dashboard filtering without reconnecting. It is never
offered to `IouKeys` at all. The Open-channel form's `iou_key` step
(below) only ever offers Keyring pick/generate — no third "use wallet"
option.

### F. Providers

A labelled store, `BTreeMap<Label, ProviderEntry { base_url, terms: Option
<Terms>, fetched_at: Option<Timestamp> }>`:

- **Auto-populated**: a successful Terms discovery (section D) against a
  `base_url` offers to save itself here under a label.
- **Re-fetch on demand**: a button per entry re-runs section D's discovery
  and replaces the cached `Terms`, so a stale cache is never silently
  trusted forever.
- **Manual add/edit**: for a provider with no reachable `.well-known` yet
  (or none at all), every `Terms` field can be hand-entered instead of
  fetched.

**Resolved — fetched `Terms` pre-fills, it doesn't lock**: a fetched
`Terms` pre-fills the providers entry (and, downstream, the
Open-channel form), but every field stays editable afterward — including
on an entry that _was_ fetched. There is one hard exception: `asset` is
locked whenever a `Terms` is attached (section J), per the no-wrong-asset
decision — everything else (`close_period`, `provider`, etc.) is editable.
Any field whose current value no longer matches the last `fetched_at`
snapshot is visually flagged ("potentially not recognized by provider") —
a warning, not a block, since the provider's own validator is the real
enforcement point regardless of what Console shows. Re-fetching replaces
the snapshot and clears the flag for fields that now match again.

**Two directions of use, both at the Rust level now** (this used to say
"neither a Rust-level dependency between `subbit-console` and
`subbit-tx`" — that boundary was revisited in section I: `subbit-console`
now depends on `subbit-tx` directly, via `Basket`, so both directions
below are plain Rust calls within one process, not JSON exchanged
between two separately-compiled wasm bundles):

- **Providers → Open form**: opening a channel against a known
  provider reads an entry (by label) to pre-fill the form, per the
  resolution above. This direction was already covered.
- **Channel → Providers** (the direction that was missing): a
  discovered on-chain channel (one of `Basket::channels()`'s entries)
  carries a bare `Constants.provider` vkh, with no label of its own.
  Giving it context
  — "this is ACME's metered-API service" — means looking that vkh up in
  the Providers. `Providers::by_provider(provider: &Hash28) -> Vec<...>`
  (wasm: `providersByProviderJson`) answers this, and deliberately
  returns a **list, not an `Option`**: one provider key can legitimately
  back more than one labelled service (distinct `Terms` — different
  catalogs, `close_period`s, etc. — under the same operator key), so a
  channel's provider vkh can match several entries at once.

**"Is Providers doing too much?" — resolved by splitting out
reconciliation, not storage.** `Providers` itself stays a pure store
(label/base_url/terms/fetched_at CRUD + the by-provider query) — that
part wasn't the problem. What was missing is a _separate_ module,
`subbit_console::reconcile`, that joins an observed `Constants` (from
`subbit-l1-dto` directly, not from `subbit-tx` — this one module
specifically doesn't need it, even though `subbit-console` as a whole now
does, via `Basket`) against whatever `by_provider` found, and reports
_divergence_ per entry, not just match/no-match: "matches ACME's key, but
this channel's `close_period` is shorter than ACME's terms require" (an
`Asset` mismatch is exact; `close_period` uses the same `>=` tolerance —
longer is safer, only _too short_ is flagged — as `close_period`'s own
semantics). This mirrors a precedent already in the codebase:
`packages/index/src/naive.rs`'s `Config`/`is_backing` keeps declared
config and observed on-chain data as two distinct types, joined only
through an explicit matcher function — the difference is `is_backing`
silently drops a mismatch (a boolean filter), where `reconcile::reconcile`
reports which fields diverge, since a UI needs to say _why_, not just
exclude it. Wasm: `JsConsole::reconcileJson(constants_json)` does the
`by_provider` lookup and the reconciliation in one call, returning the
annotated `Vec<Match>` — the comparison logic lives in Rust now, not
duplicated in JS. `constants_json` comes straight out of
`JsConsole::basketChannelsJson()`'s own per-channel output (section I)
— `basketChannelsJson`/`reconcileJson` are two different `#[wasm_bindgen]`
_methods_ on the same `JsConsole`/`Store` now (not two separate wasm
objects — the basket folded into `JsConsole` once it joined `Store`,
section G), so this is JSON-at-a-method-call, not JSON-at-a-crate-
boundary the way it was when `JsChannel` lived in `subbit-tx`.

### G. Persistence & data management

One serde-serializable blob is the unit of truth: `Store { keyring,
consumers, iou_keys, providers, whitelisted_validators }` (the last from
section K) — named `Store`, matching every other module's "module name
= its main type" convention (`keyring::Keyring`, `providers::Providers`,
`consumers::Consumers`, `iou_keys::IouKeys`).

- **Native**: a file, same `Config::load`/`save` pattern as `packages/cli/
src/config.rs` (useful for any native consumer of this same logic — dev
  tooling, tests — even though the shipped Console itself is browser-only
  per section J).
- **Wasm/browser**: Rust only (de)serializes to/from a JSON string — the
  same convention `stamp::wasm::JsStamp::set_catalog`/`catalog` already
  use for catalog data — and the plain-JS app shell owns the actual
  `localStorage.getItem`/`setItem` calls. Keeps the wasm side storage-free
  and natively testable; keeps browser-storage quirks (private-mode,
  per-origin, can be cleared by the user/browser at any time) entirely in
  JS where they're easy to reason about.
- **Data management UI**, required (not a nice-to-have given `localStorage`
  can vanish without warning):
  - **Export**: dump the current blob as a downloadable JSON file.
  - **Import**: file picker → JSON → validated via a wasm parse/decode
    call (section H: never half-apply a corrupt import) → replaces the
    current blob.
  - **Delete all data**: confirmation dialog ("Delete data? This cannot be
    undone.") → clears only Console's own `localStorage` key(s), not the
    whole origin.

### H. Error handling

- One top-level `Error` enum (`thiserror`) for all of Console's own new
  Rust logic (Keyring ops, Providers ops, Basket ops — wrapping whatever
  `subbit_tx::tx::Error`/`BuildError` says via its own `Display`, not
  re-deriving a parallel set of variants — Open-form validation such as
  duplicate-tag/min-iou checks, import/export (de)serialization) —
  consolidating the per-module pattern already used elsewhere (`claim::
keyring::Error`, `stamp::stamp::ResponseError`) into one enum for this
  one new surface, rather than splitting it further.
- **Wasm must never panic.** Every `#[wasm_bindgen]`-exposed fn returns
  `Result<_, JsError>`, mapped at the boundary exactly like `stamp::wasm::
JsStamp::response` already does (`.map_err(|e| JsError::new(&e.to_string
()))`) — never `.unwrap()`/`.expect()`/`panic!` on anything crossing from
  JS (user input, fetched `Terms`, parsed UTxOs, imported JSON, wallet
  responses), since a wasm panic poisons the module for the rest of the
  page session, not just the one call.

### I. Basket — shopping-trolley tx building, unified core

Console's tx-building UX is a basket, not a one-action-one-tx flow — built
directly on `subbit-tx::Tx`'s existing API (`tx.rs`), which already _is_ a
basket: `wills: BTreeMap<Input, Will>` (staged intents against existing
channels) + `opens: BTreeMap<String, Open>` (staged new-channel opens),
with `propose`/`drop_intent`/`drop_all_intents` and `add_open`/`drop_open`/
`drop_all_opens`, and a single `build(...)` that assembles _everything_
currently staged into one transaction. This mirrors `subbit-cli`'s own
existing stage → propose → submit flow (`packages/cli/src/tx.rs`,
`tx/forms.rs`, `tx/select.rs` — see Context, top of this doc).

**Revised: this basket is `subbit_console::Basket`, not a `subbit-tx`
wasm type.** The first version of this section put the basket behind
`subbit-tx`'s own (now-removed) `wasm` feature, directly wrapping `Tx`
there. That kept the fetcher (Floe) and the tx-builder _external_ to
Console's own core — exactly backwards from "what we want is a unified
core with thin wrappers to make it Native or Wasm." `Basket` (`packages/
console/src/basket.rs`) wraps `subbit_tx::tx::Tx` directly — a Rust
dependency on `subbit-tx`, not JSON across a wasm boundary — so the
_same_ basket logic backs both `src/wasm.rs` (the browser, via
`JsConsole`'s own `basket*` methods — see below, folded into `JsConsole`
rather than a separate `JsBasket`) and `src/main.rs` (the native
"test-driving" bin's `basket-open`/`basket-drop-open`/`basket-list`
commands, and any future native TUI wanting "an interface very similar
to the web-app"). `subbit-tx` stays entirely `wasm_bindgen`-free — a
pure logic crate, same as it always was for `subbit-cli`/`subbit-claim`/
`subbit-canary`'s own native use.

Every `Basket` method takes already-typed domain values (`Input`,
`Want`, `Constants`, a typed Floe `Utxo`, a typed `(Input, Output)` fee
map), never raw hex/JSON strings — parsing those belongs to whichever
frontend is crossing _its own_ boundary (`src/wasm.rs` parsing JS
strings and CIP-30's hex-CBOR; a native wrapper parsing CLI args, or
reading already-typed values straight out of a real `Backend`/wallet
session with no decoding needed at all), never to the shared core. This
is also why `Basket::add_open` takes an already-built `Constants`
rather than six raw hex strings the way the original `wasm.rs` draft's
`addOpen` did — building `Constants` from raw input is itself a frontend
concern now.

Uses the codebase's existing three-stage vocabulary as-is (`packages/tx/
src/step.rs`), no new terms invented:

- **`Can`**: advisory, derived from a channel's current state (what
  section J's table shows per row — Add/Close/Elapse/End availability).
- **`Want`**: the user's flat, unconstrained expression of intent — "add
  to basket" wraps `Basket::propose(input, want)` for an existing
  channel, or `Basket::add_open(constants, amount, delegation)` for a
  new one.
- **`Will`**: the validated result, cached inside `Tx` (and so inside
  `Basket`) once `propose`/`add_open` succeeds — what the basket panel
  actually lists and what `drop_intent`/`drop_open` remove.

**Basket panel**: lists every staged `Will`/`Open` (`Basket::channels`/
`Basket::opens`, provider, action, key figures), each removable
individually (`drop_intent`/`drop_open`) or all at once
(`drop_all_intents`/`drop_all_opens`) before checkout — nothing is sent
anywhere until the user explicitly checks out.

**Checkout**: one `Basket::build(network, reference_utxo,
change_address, fuel)` call assembles every staged `Will`/`Open` into a
single `Transaction`, CBOR bytes handed to CIP-30's `signTx` +
`submitTx` (wasm) — a native wrapper would hand the same `Transaction`
to a real wallet session's own signing instead. On failure, `build`
leaves the basket completely unchanged (`Tx::build`'s own documented
guarantee, inherited for free) — nothing is cleared until a transaction
is actually produced, so a failed checkout is always safely retriable.

**Resolved — the basket is part of `Store` now.** `Basket` derives
`Serialize`/`Deserialize` exactly as `Tx` itself already does (the same
shape `subbit-cli`'s own `Config::cache_path` already relies on to
cache a staged `Tx` between CLI invocations — this isn't a new pattern,
just reused), and `Store` (section G) carries it as a field, persisted
through the exact same native-file/wasm-JSON/Export-Import path as
everything else. Across a Floe refresh, every staged `Open` survives
unconditionally (`Basket::refresh_channels`); staged `Want`s against
_existing_ channels do not — `Tx`'s own `wills` retain only the
validated `Will`, not the original `Want`, so there's nothing safe to
re-`propose` against refreshed channel state, and carrying the old
`Will` forward unchecked would risk building against stale state. A
deliberate, scoped limitation, not an oversight: re-stage those after a
refresh if still wanted. Verified with a real native CLI round-trip
(`basket-open` in one process, `basket-list` in a separate one, same
staged `Open` both times) and a `Store`-level TOML round-trip test.

**Lower priority / stretch**: `packages/canary/src/l1.rs` already has a
robust multi-want batching strategy — stage a batch, submit, and on
rejection (e.g. one stale `Want` in the batch) back off to a smaller
sub-batch and retry rather than failing the whole batch — see `l1.rs`'s
`flush`/`submit_steps` and `open_batch`. Mirroring that in Console (so an
over-sized or partially-stale basket degrades gracefully into several
chained transactions instead of one hard failure) is valuable but explicitly
not required for this MVP; noted in "Explicitly out of scope."

### J. `packages/console/www/`: the app

Plain JS, no framework, no bundler — same convention as `examples/echo/www`
(`wasm-bindgen --target web` output is a directly-importable ES module via
`<script type="module">`). Not a new Cargo workspace member; this directory
holds only the static site, composing **one** wasm-bindgen bundle (down
from the three-bundle split this section originally described, then the
two-bundle split section C's own earlier draft had — see section C's
current text: `floe-client` is now embedded directly, no separate bundle
at all) plus CIP-30:

- **`subbit-console` wasm pkg** (`nix/console-www.nix`, sections A, C,
  E-J): one `JsConsole` plus two free async fns (`fetchLiveChannels`/
  `fetchHistory`, section C) that reach Floe directly via Console's own
  embedded `Backend`. `fetchLiveChannels` + `basketRefreshChannelsJson`
  renders the dashboard table: provider, amount, stage, available
  actions (Add/Close/Elapse/End, each "add to basket"); `fetchHistory` +
  `reconstructLinesJson` backs the lines/finality view. Identity/provider
  bookkeeping (sections E-H) sits alongside, all backed by the one
  persisted `Store` (section G). The validator's reference-script UTxO
  (needed by `basketBuild`) still needs its own fetch-then-filter step —
  `fetchLiveChannels` against a known `ref_script_host` address, then
  `subbit_tx::validator::find_reference_script` over the result — not
  yet wired into a dedicated wasm fn; noted in "Open items/risks".
- **CIP-30** (vanilla JS): `signTx` + `submitTx`.

**Dashboard-first rendering**: the table above is built from _every_
channel Floe discovers at the validator's payment credential — unfiltered,
no keyring and no wallet required. When `Consumers`/`IouKeys` (section E)
have entries and/or a wallet is connected, their vkhs/iou_keys become a
preferred filter/highlight ("mine") layered on the same table, not a
precondition for populating it.

**Lines + finality per channel, not just current state** — done, by
_reusing_ `subbit-index` directly rather than porting its algorithm.
The dashboard shows a channel's history as **lines** (plural — the same
term, same shape, as `subbit-index`'s own `Line(Vec<Item>)`/`Channel{
constants_ext, lines}` server-side model), annotated with whether each
item is _supposedly final_ yet, per the matched Terms' `confirmation_depth`
("supposedly" since this is a depth heuristic Console computes itself,
not a protocol guarantee): `ledger_tip.height - item.block_ref.height >=
Terms.confirmation_depth`.

Reuse, not reimplementation, required one real fix along the way.
`subbit-index`'s `cardano.rs`/`lineage.rs` were typed concretely against
`floe_client::grpc::Client` rather than the already-existing
`floe_client::Backend` trait — genericizing them (`&mut impl Backend`)
was the easy, safe part (no behavior change, every existing test still
passes, `canary` — `subbit-index`'s only other native lib consumer —
unaffected once its `Cargo.toml` requests the right feature). **The
tempting next step was wrong, though**: making a `subbit-index` `wasm`
feature forward to `floe-client/wasm` so the same generic functions could
run against Floe's wasm transport directly. That fails for a structural
reason, confirmed by actually hitting it: `floe-client`'s own internal
workspace pins an exact `wasm-bindgen` version that differs from ours.
**Superseded by section C's own current text**, though: the fix wasn't
to give up on reusing `floe-client/Backend` from wasm, but to supply
_our own_ `Backend` impl under our pin instead of `floe-client`'s —
`subbit_console::wasm::transport::Client`. `search_all`/`search_history`
now run directly against Console's wasm build via that impl; no JSON
hand-off from a separate bundle needed.

Also genuinely useful regardless: `lineage::build_output` is split into
a pure, `async`/`Backend`-free `lineage::reconstruct(utxos: Vec<Utxo>,
ledger_tip: ChainPoint) -> Output`, with `build_output` now just
`cardano::search_history` + `reconstruct` for native callers. Console's
wasm build depends on `subbit-index` (bare, no `native`/`grpc` feature)
and calls `cardano::search_history` (via `src/floe.rs`'s
`discover_history`, section C) + `reconstruct` directly — the exact
same reconstruction code `subbit-index`'s native lineage-index service
uses, not a port. Tie-break rules for mimicked/ambiguous chains
(`build_lines`'s own sort/dedup logic) are inherited for free, already
tested by `subbit-index`'s own existing test suite.

**Wallet-gated actions**: the per-row "add to basket" buttons and "Open
new channel" are visible regardless, but disabled until a CIP-30 wallet is
connected — browsing never requires a wallet; staging into the basket and
checking out always does (checkout needs the wallet's fee UTxOs and
signature regardless of basket contents).

**Open-channel form** (stages an `Open` into the basket, section I, rather
than building immediately):

- **Provider**: pick a `Providers` entry (section F — re-fetches or uses
  cache), or "add manually" (free-entry `Terms` fields for a provider with
  nothing published yet; saved into `Providers` as a never-fetched
  entry).
- **iou_key**: pick an existing Keyring label, or "generate new" (section
  E — immediately saved into the Keyring under a user-given label). No
  "use wallet" option — see section E's resolution.
- **Tag**: freely chosen or generated; a non-blocking warning appears if
  it collides with a tag already known for the same iou_key, but
  submission is permitted (legitimate reuse, per section E).
- **Asset**: locked to `Terms.asset` whenever a `Terms` is attached (from
  Providers or a fresh fetch) — not editable, so there is no way to
  submit a mismatched asset class. Free-entry only in manual mode, where
  there's nothing to validate against.
- **Min-IOU warning**: compares the connected wallet's held balance of
  `Terms.asset` against `Terms.min_iou_amount`; shows a non-blocking
  warning (not a hard block) if insufficient.
- On submit: adds the `Open` to the basket (section I) and always offers
  the resulting L2 Channel export (section E), regardless of which iou_key
  path was used — export isn't deferred to checkout, since the user needs
  it for Stamp whether or not/whenever the basket is eventually checked
  out.

### K. Multi-network config

Since both preprod and mainnet ship from the start: a small per-network
table in the app config (Floe endpoint, `ref_script_host` address, which
`VALIDATOR`/script hash to expect) the user picks between, with `Terms.
network` used as a sanity check against whichever one is selected before
building anything.

A **whitelisted-validators** list (persisted, section G) lets the user add
script hashes beyond the current build's own `VALIDATOR.hash`, which is
included by default. Discovery (section J) runs against every whitelisted
validator, not just the build's own.

## Explicitly out of scope for this MVP

- `Sub`/`Settle` actions (provider-only).
- Running/embedding Stamp's own wasm logic inside Console (data-only
  interchange, section E). Note that `floe` may _require_ a Subbit channel!
  so do not exclude this case.
- Standing up or CORS-enabling a public Floe endpoint (assumed to exist).
- PWA installability — noted as a later goal; nothing in this plan
  precludes it (a static site can always gain a manifest + service worker
  later), so no extra work now.
- Dashboard view for channels at validators _not_ on the whitelist
  (section K) — out of scope, not a gap: unknown validators are
  deliberately not discovered.
- Multi-tx checkout: a basket's `build()` always produces one transaction
  (section I). Splitting an over-sized basket across multiple chained txs
  (mirroring `canary`'s batch-backoff-and-retry in `l1.rs`) is noted as a
  lower-priority stretch, not MVP.

## Open items / risks

- **Governing rule for every `wasm.rs` in this plan, restated after
  getting it wrong once**: `wasm.rs` holds no logic and no tests, ever —
  only hex/JSON marshalling plus a call into an already-typed, already-
  tested function living in a core module (or reused directly from
  `subbit-tx`/`subbit-index`). While wiring `reconstructLinesJson`
  (section J), a private helper + its own test module briefly ended up
  living inside `subbit-console`'s `wasm.rs` instead — motivated by a
  real, separate problem (`JsError::new` is a wasm-bindgen _imported_
  function, a real binding into the JS host, and panics if called
  outside an actual wasm32+JS environment, so a `Result<_, JsError>`
  function can't be natively tested end-to-end even on its success path)
  but the wrong fix for it: it solved "how do I get _a_ test for this
  wasm function" rather than asking whether the wasm function had any
  console-owned logic worth testing at all. It didn't — `reconstruct` is
  tested in `subbit-index`, and `serde_json`'s (de)serialization of a
  type this crate doesn't own isn't ours to re-test. Corrected: no
  helper, no test, `reconstruct_lines_json` is back to a few lines of
  pure marshalling, consistent with every other function in that file.
  The `JsError`-panics-natively fact above is still real and still
  applies the moment a wasm-facing function _does_ need to wrap genuine
  console-owned logic — the actual fix in that case is to put the logic
  (and its test) in a core module in the first place, not to tuck a
  private `JsError`-free clone of it inside `wasm.rs`. `subbit-tx`'s
  earlier `wasm.rs` (section A) _did_ turn out to need exactly this
  audit — it had no core-module split at all (`JsChannel`/`JsTx`'s logic
  lived directly in `wasm.rs`, verified only by `cargo check`/`clippy`
  on wasm32, no native unit tests) — but the fix ended up being bigger
  than "move the logic into a `subbit-tx` core module": that whole wasm
  surface was removed and absorbed into `subbit-console`'s `Basket`
  instead (section I), which already follows this rule (`JsConsole`'s
  `basket*` methods in `wasm.rs` are pure marshalling; the logic + its
  tests live in `basket.rs`).
- ~~`floe-client`'s wasm feature's `crate-type` declaration is unverified~~
  — **resolved**: `[lib] crate-type = ["cdylib", "rlib"]` is already
  declared unconditionally in `floe-client`'s `Cargo.toml`; `cargo check -p
floe-client --no-default-features --features wasm --target
wasm32-unknown-unknown` builds clean (verified standalone against the
  checked-out sibling repo). No PR to `kompact-io/floe` needed.
- `getrandom`'s wasm backend for in-browser keygen (section E's "Generate"
  path) needs a real build+run smoke test, not just `cargo check` — the
  original tx-builder plan flagged this as low-risk but never verified it.
- No independent way to confirm a CIP-30-submitted tx landed except polling
  Floe's `read_tx` (implemented) or re-polling `search_utxos` — fine for
  v1, not building retry/backoff logic speculatively.
- Terms' fallback well-known paths (beyond `.well-known/subbit` itself) are
  a short, to-be-defined list — not pinned down yet.
- `localStorage` persistence (section G) has no cross-device/cross-browser
  sync and can be cleared by the browser or user at any time — Export/
  Import is the only mitigation in this MVP; no cloud backup.
- Basket checkout (section I) assumes one `Tx::build()` call handles
  everything staged; no size/fee-limit check on the basket before build,
  so a large basket's first signal of trouble is `BuildError::Balancing`
  or an oversized-tx rejection at submission, not an earlier warning.
- ~~Basket persistence (section I) is unresolved~~ — **resolved**:
  `Basket` derives `Serialize`/`Deserialize` (same shape `Tx` already
  had, reused, not reinvented) and is now a `Store` field, persisted
  through the same native-file/wasm-JSON/Export-Import path as
  everything else. Staged `Open`s survive a Floe refresh
  unconditionally; staged `Want`s against existing channels don't (see
  section I's own resolved note for why that's a deliberate, scoped
  limitation).

## Verification

- `cargo check -p subbit-tx --no-default-features --features "serde
floe"` (native and `--target wasm32-unknown-unknown`) — done. (No
  `wasm` feature anymore — see section A's revision.)
- `cargo check -p subbit-console --no-default-features --features wasm
--target wasm32-unknown-unknown` — done; covers `Basket` and the
  embedded `floe-client` `Backend` impl (section C) too now.
- `nix build .#subbit-console-wasm`, `.#console-www` — done once, as a
  one-off sanity check that the single-bundle embedding (section C)
  resolves and links under this workspace's own `Cargo.lock`/`craneLib`
  (it does — `floe-client`'s own `wasm` feature is never reached). Not
  the primary verification loop going forward, by request — day-to-day
  work here now checks via plain `cargo check`/`build`/`test` (debug
  profile), not `nix build`.
- `cargo test -p subbit-console` (native, and again with `--features
wasm`, no `--target wasm32-unknown-unknown` needed for the test run
  itself — see the `JsError`-panics-natively finding below) — done;
  covers `Basket` (open + build produces a real signable `Transaction`;
  `refresh_channels` keeps `Open`s, drops `Will`s; JSON round-trip),
  `Keyring`/`Consumers`/`IouKeys`'s cross-container invariant,
  `Providers::by_provider`, `reconcile`, `finality`, `Store` round-trips
  (including a dedicated TOML round-trip for `Store.basket`, the
  riskiest path given `Input`/non-string-keyed maps).
- `cargo run -p subbit-console -- basket-open ...` in one process,
  `basket-list` in a **separate** process invocation against the same
  `--path` — done; the staged `Open` survives, confirming `Store.basket`
  persistence actually works end to end, not just that it compiles —
  and proves the "unified core, thin native/wasm wrappers" shape, since
  this is a real native invocation, no wasm involved at all.
- Manual smoke test against preprod: connect a CIP-30 test wallet (Eternl/
  Lace), fetch Terms from a real provider base_url, open a channel with a
  freshly generated account, confirm it's discoverable afterward via the
  same Floe `search_utxos` call, and that Add/Close build txs the wallet
  will sign.
- Manual smoke test, no wallet connected: load Console cold (fresh
  `localStorage`), confirm the channel table still populates from Floe and
  tx-building buttons are disabled; connect a wallet mid-session and
  confirm the same table re-highlights without a reload.
- Manual smoke test, data management: Export the keyring/providers blob,
  clear data via "Delete all data", confirm an empty state, Import the
  exported blob back, confirm Keyring/Consumers/IouKeys/Providers are
  restored exactly.
- Manual smoke test, error handling: feed a corrupt/truncated Import blob
  and a malformed pasted verifying key; confirm both surface as a visible
  error with no browser-console panic/trap and no partial state change.
