import init, {
  JsConsole,
  decodeAddressVkh,
  fetchHistory,
  findReferenceScriptJson,
  defaultValidatorHash,
} from "./pkg/subbit_console.js";

await init();

const CONFIG_KEY = "subbit-console.config";

let config = loadConfig();
// `Ctx::import`/`export` read/write `localStorage` directly on wasm32
// now (see `ctx.rs`) -- no JSON string crosses into JS for this, unlike
// the explicit "download/upload a backup file" Data Management actions
// (`toJson()`/`new JsConsole(text)`, still string-based on purpose).
let store;
try {
  store = JsConsole.load();
} catch (e) {
  log(`store load failed, starting fresh: ${e}`);
  store = new JsConsole(undefined);
}
let wallet = null; // { vkh, addrHex, networkId } -- the CIP-30 connection's own display details, session-only
// Which backend Rust's own `JsConsole.wallet` (a single slot -- see
// `ui::wasm::WalletHandle`'s own doc) is currently holding for
// submit -- `null`, `"cip30"`, or `"embedded"`. Connecting one backend
// silently replaces whatever the other had connected, so this (and
// `wallet` above) exist only to keep this file's own display in sync
// with that, never to decide anything submit itself needs.
let submitBackend = null;
// The last `fetchHistory` response -- lets a Keys/Settings change
// re-render the Channels tab ("mine?"/actions) without a Floe refetch,
// same idea `renderDashboardFromCache` used to serve. `null` until the
// first fetch.
let lastHistoryJson = null;

function loadConfig() {
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      // Older configs kept `network` and `floeBaseUrl` as two
      // independent fields (a network picker feeding a per-network
      // url map). Floe doesn't report its own network (nothing in its
      // wire protocol carries one), so that's now said once, as part
      // of the one Floe setting, rather than picked separately --
      // an old-shaped config is just treated as unset.
      if (parsed.floe) return parsed;
    }
  } catch (e) {
    log(`config load failed, using defaults: ${e}`);
  }
  return { floe: { baseUrl: "https://floe.kompact.io", network: "preprod" } };
}

function saveConfig() {
  localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
}

function saveStore() {
  store.save();
}

function log(msg) {
  const div = document.createElement("div");
  div.textContent = `[${new Date().toISOString()}] ${msg}`;
  document.getElementById("log").prepend(div);
}

// Drains whatever `crate::Logs` (Rust) pushed into `store` since the
// last drain, rendering each entry the same way a direct `log(...)`
// call does -- `ui::cli`'s equivalent command narrates the exact same
// wording, flushed to stderr instead of this div. Only a handful of
// `store.*` calls push anything here (see `ui::wasm`'s own doc); most
// actions still log directly on this side, same as ever.
function drainLogs() {
  for (const entry of JSON.parse(store.logsEntriesJson())) {
    log(entry.level === "Error" ? `[error] ${entry.message}` : entry.message);
  }
  store.logsClear();
}

function el(id) {
  return document.getElementById(id);
}

// --- Input validation ------------------------------------------------------
//
// Client-side feedback only -- Rust still re-validates on every `*Add`
// call and is the actual source of truth (same "render, don't decide"
// split as everywhere else in this file); this exists so a malformed
// hex string gets a specific, immediate reason shown right next to the
// field, instead of only surfacing as a generic failure after a round
// trip through a rejected Rust call.

function setFieldError(input, message) {
  input.classList.toggle("invalid", !!message);
  let err = input.nextElementSibling;
  if (!err?.classList.contains("field-error")) {
    err = document.createElement("div");
    err.className = "field-error";
    input.after(err);
  }
  err.textContent = message ?? "";
  err.style.display = message ? "block" : "none";
}

function clearFieldError(input) {
  setFieldError(input, "");
}

// `byteLen` in bytes (hex string is twice that many characters).
// `optional`: an empty value is valid (nothing to add yet), used for
// fields a caller only fills in for one of several buttons sharing it
// (e.g. Keyring's key field: required for "Add existing", ignored by
// "Generate new").
function validateHex(input, byteLen, { optional = false } = {}) {
  const value = input.value.trim();
  if (value === "") {
    setFieldError(input, optional ? "" : "required");
    return optional;
  }
  if (!/^[0-9a-fA-F]+$/.test(value)) {
    setFieldError(input, "must be hex (0-9, a-f)");
    return false;
  }
  if (value.length !== byteLen * 2) {
    setFieldError(
      input,
      `must be ${byteLen} bytes (${byteLen * 2} hex chars) -- got ${Math.ceil(value.length / 2)}`,
    );
    return false;
  }
  clearFieldError(input);
  return true;
}

function validateNonEmpty(input) {
  const ok = input.value.trim() !== "";
  setFieldError(input, ok ? "" : "required");
  return ok;
}

function validateUrl(input) {
  const value = input.value.trim();
  if (value === "") {
    setFieldError(input, "required");
    return false;
  }
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:")
      throw new Error();
  } catch {
    setFieldError(input, "must be a valid http(s) URL");
    return false;
  }
  clearFieldError(input);
  return true;
}

// Live feedback as the user types/leaves the field -- `validator` is
// one of the functions above, pre-bound to its own field-specific
// arguments by the caller.
function liveValidate(input, validator) {
  input.addEventListener("input", () => validator());
  input.addEventListener("blur", () => validator());
}

// One row per item, each with its own "Remove" button -- the shared
// shape every Keyring/Consumers/IouKeys/Providers/Whitelist list
// renders into, now that each needs per-item removal, not just a
// read-only dump.
function renderItemList(container, items, renderText, onRemove) {
  container.innerHTML = "";
  if (items.length === 0) {
    container.textContent = "(none)";
    return;
  }
  const ul = document.createElement("ul");
  ul.className = "item-list";
  for (const item of items) {
    const li = document.createElement("li");
    const span = document.createElement("span");
    span.className = "item-text";
    span.textContent = renderText(item);
    const removeBtn = document.createElement("button");
    removeBtn.textContent = "Remove";
    removeBtn.addEventListener("click", () => onRemove(item));
    li.append(span, removeBtn);
    ul.appendChild(li);
  }
  container.appendChild(ul);
}

// --- Tabs ----------------------------------------------------------------

function activateTab(name) {
  for (const btn of document.querySelectorAll("#tabs button")) {
    btn.classList.toggle("active", btn.dataset.tab === name);
  }
  for (const panel of document.querySelectorAll(".tab-panel")) {
    panel.classList.toggle("active", panel.id === `tab-${name}`);
  }
}
for (const btn of document.querySelectorAll("#tabs button")) {
  btn.addEventListener("click", () => activateTab(btn.dataset.tab));
}

// --- Settings tab: Floe backend --------------------------------------------
//
// One setting, not two: Floe can't report which network it serves (no
// field anywhere in its wire protocol carries one), so the network is
// said here, alongside the base URL, rather than picked independently
// -- Console trusts this value for the embedded wallet's address and
// for catching a connected CIP-30 wallet on the wrong network.

el("floe-base-url").value = config.floe.baseUrl;
el("floe-network").value = config.floe.network;

function renderConnectionStatus() {
  const status = el("connection-status");
  if (!config.floe.baseUrl) {
    status.textContent = "Floe: not configured -- click to set up";
    status.className = "unset";
    status.onclick = () => {
      activateTab("settings");
      el("floe-base-url").focus();
    };
    return;
  }
  let host = config.floe.baseUrl;
  try {
    host = new URL(config.floe.baseUrl).host;
  } catch {
    // Not a parseable URL yet (mid-edit) -- show it raw rather than error.
  }
  status.textContent = `Floe: ${host} (${config.floe.network})`;
  status.className = "ok";
  status.onclick = () => activateTab("settings");
}
renderConnectionStatus();

// CIP-30's `getNetworkId()`: 0 = testnet (preprod), 1 = mainnet.
function networkMismatch(walletNetworkId) {
  const expected = config.floe.network === "mainnet" ? 1 : 0;
  return walletNetworkId !== expected;
}

el("floe-base-url").addEventListener("change", () => {
  config.floe.baseUrl = el("floe-base-url").value.trim();
  saveConfig();
  renderConnectionStatus();
});

el("floe-network").addEventListener("change", () => {
  config.floe.network = el("floe-network").value;
  saveConfig();
  renderConnectionStatus();
  renderEmbeddedWallet(); // the shown address is network-dependent
  if (wallet && networkMismatch(wallet.networkId)) {
    wallet = null;
    submitBackend = null;
    store.disconnectWallet();
    el("wallet-connection-out").textContent = "(not connected)";
    el("basket-submit").disabled = true;
    log("wallet disconnected: no longer matches the configured network");
  }
});

// --- Settings tab: whitelisted validators ---------------------------------

function renderWhitelist() {
  const hashes = JSON.parse(store.whitelistJson());
  renderItemList(
    el("whitelist-out"),
    hashes,
    (hash) => hash,
    (hash) => {
      store.whitelistRemove(hash);
      saveStore();
      renderWhitelist();
      log(`removed whitelisted validator ${hash}`);
    },
  );
}

// Seed with the build's own validator hash on first run.
if (JSON.parse(store.whitelistJson()).length === 0) {
  store.whitelistAdd(defaultValidatorHash());
  saveStore();
}
renderWhitelist();

liveValidate(el("whitelist-hash"), () => validateHex(el("whitelist-hash"), 28));

el("whitelist-add").addEventListener("click", () => {
  const hashInput = el("whitelist-hash");
  if (!validateHex(hashInput, 28)) return;
  try {
    store.whitelistAdd(hashInput.value.trim());
    saveStore();
    hashInput.value = "";
    renderWhitelist();
  } catch (e) {
    log(`whitelist add failed: ${e}`);
  }
});

// --- Settings tab: CIP-30 wallet connection --------------------------------

function detectWallets() {
  const select = el("wallet-select");
  select.innerHTML = "";
  const cardano = window.cardano ?? {};
  for (const key of Object.keys(cardano)) {
    if (!cardano[key]?.enable) continue;
    const opt = document.createElement("option");
    opt.value = key;
    opt.textContent = cardano[key].name ?? key;
    select.appendChild(opt);
  }
}
detectWallets();

el("wallet-connect").addEventListener("click", async () => {
  const key = el("wallet-select").value;
  if (!key) {
    log("wallet connect: no CIP-30 wallet detected");
    return;
  }
  try {
    const api = await window.cardano[key].enable();
    const networkId = await api.getNetworkId();
    if (networkMismatch(networkId)) {
      throw new Error(
        `wallet is on ${networkId === 1 ? "mainnet" : "testnet"}, Floe is configured for ${config.floe.network}`,
      );
    }
    const usedAddrs = await api.getUsedAddresses();
    const unusedAddrs = usedAddrs.length ? [] : await api.getUnusedAddresses();
    const addrHex = usedAddrs[0] ?? unusedAddrs[0];
    if (!addrHex) throw new Error("wallet has no addresses");
    const vkh = decodeAddressVkh(addrHex);
    // `signTx`/`submitTx`/`getUtxos`/`getChangeAddress` now go through
    // Rust (`store.submit`, via `subbit_wallet::Wallet`) -- `api` is
    // handed over, not kept here for this file to call directly any
    // more.
    // `vkh` -- Rust has no visibility into the CIP-30 `api` object's
    // own addresses, so this is the one piece of signer identity this
    // file still has to hand over itself (`Ctx::wallet_vkh` derives
    // the embedded wallet's own vkh straight from key material it
    // already holds; there's no such shortcut for a live CIP-30
    // session -- see `ui::wasm`'s own `cip30_vkh` doc).
    store.connectCip30(api, vkh);
    wallet = { vkh, addrHex, networkId };
    submitBackend = "cip30";
    el("wallet-connection-out").textContent = `connected: ${key}\nvkh: ${vkh}`;
    el("basket-submit").disabled = false;
    log(`wallet connected, vkh=${vkh}`);
  } catch (e) {
    log(`wallet connect failed: ${e}`);
  }
});

el("wallet-disconnect").addEventListener("click", () => {
  wallet = null;
  submitBackend = null;
  store.disconnectWallet();
  el("wallet-connection-out").textContent = "(not connected)";
  el("basket-submit").disabled = true;
  log("wallet disconnected");
});

liveValidate(el("wallet-consumer-label"), () =>
  validateNonEmpty(el("wallet-consumer-label")),
);

el("wallet-save-consumer").addEventListener("click", () => {
  if (!wallet) {
    log("save as consumer: no wallet connected");
    return;
  }
  const labelInput = el("wallet-consumer-label");
  if (!validateNonEmpty(labelInput)) return;
  try {
    const label = labelInput.value.trim();
    store.consumersAdd(label, wallet.vkh);
    saveStore();
    labelInput.value = "";
    renderConsumers();
    renderLineageFromCache();
    log(`saved connected wallet's vkh as consumer ${label}`);
  } catch (e) {
    log(`save as consumer failed: ${e}`);
  }
});

// --- Channels tab: lineage, finality, and live actions -----------------------
//
// One widget per Constants (`channelsLineageJson`'s own organization,
// `index-dto`'s, not re-grouped by keytag). One Floe fetch
// (`fetchHistory`) backs this whole tab: the per-Constants widgets
// (keys resolved to labels + Terms-match + `mine`, via `brief`), each
// line's own live actions (`LineView.can`, `None` once its tip is no
// longer live), and (filtered to unspent, client-side) submit's own
// reference-script lookup. `channelsLineageJson` (Rust,
// `Ctx::channels_lineage`) already decides grouping/finality/mine/can/
// labels -- joined server-side against the live txo cache
// (`basket_refresh_txos` fills it) rather than this file fetching a
// separate dashboard and joining it by txo itself, which is how this
// used to work (see `crate::channels::status`'s own doc on why that
// decision moved upstream). This file only renders what it's handed.

function stagishLabel(stage) {
  if (stage === "Closed" || stage === "Settled" || stage === "Terminated")
    return stage;
  if (stage.Opened)
    return `Opened (amount ${stage.Opened.amount}, subbed ${stage.Opened.subbed})`;
  return JSON.stringify(stage);
}

function canAction(can, name) {
  return !!can && can.some((c) => c === name || c[name]);
}

// `Can::Elapse{after}`'s own on-chain-valid lower bound -- the exact
// value to submit as `Want::Elapse{lower}`, not a client-clock guess
// (`Date.now()`): Rust already decided the bound, this just reads it
// back rather than re-deciding it.
function elapseAfter(can) {
  const entry = can && can.find((c) => c && c.Elapse);
  return entry ? entry.Elapse.after : Date.now();
}

function actionButton(label, enabled, onClick) {
  const btn = document.createElement("button");
  btn.textContent = label;
  btn.disabled = !enabled;
  btn.addEventListener("click", onClick);
  return btn;
}

function stageWant(txHash, index, want) {
  try {
    store.basketPropose(txHash, BigInt(index), JSON.stringify(want));
    saveStore();
    renderBasket();
    log(`staged ${JSON.stringify(want)} against ${txHash}#${index}`);
  } catch (e) {
    log(`stage failed: ${e}`);
  }
}

function sameTxo(a, b) {
  return !!a && !!b && a.tx_hash === b.tx_hash && a.index === b.index;
}

function assetLabel(asset) {
  return asset === "Lovelace" ? "Lovelace" : `native ${asset.Native.name}`;
}

// `Stagish::Closed`/`Stagish::Settled` distinguish the two real
// on-chain stages directly (`subbit_index::lineage::to_stagish` maps
// `Stage::Settled` to its own variant, no longer folded into `Closed`
// -- `Settle` is the only action that continues a `Closed` txo into
// `Settled`; `Elapse`/`End` are both terminal, no-continuation spends,
// so a tip can go: Opened -> Closed -(Settle)-> Settled, with `Elapse`
// ending it straight from `Closed` or `End` ending it straight from
// `Settled`). So `current.stage` alone says which of the two it last
// was; whether that's still live (awaiting the next action) or already
// spent (done) is the one thing lineage history still can't say --
// that's exactly what `line.can` being `None` means (`Ctx::
// channels_lineage`'s own doc), so this function renders `can`, it
// doesn't re-derive liveness from it.
function describeCurrent(current, can) {
  if (!current) return { text: "(no items)", cls: "terminal" };
  if (current.stage === "Terminated")
    return { text: "Terminated", cls: "terminal" };
  if (current.stage.Opened) {
    return { text: stagishLabel(current.stage), cls: "pending" };
  }
  if (current.stage === "Settled") {
    return can
      ? { text: "Settled -- ready to end", cls: "pending" }
      : { text: "Settled (done)", cls: "terminal" };
  }
  // Stagish::Closed from here.
  if (!can) return { text: "Closed (done)", cls: "terminal" };
  if (canAction(can, "Elapse"))
    return { text: "Closed -- awaiting elapse", cls: "pending" };
  return { text: "Closed", cls: "pending" };
}

function renderBrief(brief, c) {
  const div = document.createElement("div");
  div.className = "brief";
  const providerText = brief.provider_matches.length
    ? brief.provider_matches
        .map(
          (m) =>
            `${m.label} (${m.has_terms ? "Terms known" : "Terms unknown"}${
              m.divergences.length
                ? `, diverges: ${m.divergences.join(", ")}`
                : ""
            })`,
        )
        .join("; ")
    : `${c.provider.slice(0, 12)}… (unlabelled)`;
  div.innerHTML = `
    <div>consumer: ${brief.consumer_label ?? `${c.consumer.slice(0, 12)}… (unlabelled)`}${brief.mine ? " · mine" : ""}${brief.mine && !brief.can_sign ? " (no signing key held -- actions below stay disabled)" : ""}</div>
    <div>iou_key: ${brief.iou_key_label ?? `${c.iou_key.slice(0, 12)}… (unlabelled)`}</div>
    <div>provider: ${providerText}</div>
    <div>tag ${c.tag} · ${assetLabel(c.asset)}</div>
  `;
  return div;
}

// `can.join(", ")` on a mixed array of plain-string unit variants
// (`"Add"`/`"Close"`/`"End"`) and one-field objects (`{Elapse:
// {after}}`) -- label each the same way `stagishLabel` already does
// for `Stagish`, rather than letting a bare object stringify as
// `[object Object]`.
function canLabel(can) {
  return can
    .map((c) => (typeof c === "string" ? c : Object.keys(c)[0]))
    .join(", ");
}

// One Line, almost always the only one under its Constants. Renders
// the two notable Txos (`current`/`finalized`, very often the same
// one) as a compact summary with live actions, plus the full item
// history underneath (collapsed by default) -- see `lineage.rs`'s own
// doc on why both forms exist.
//
// `canSign` (`brief.can_sign`): whether *this console* can actually
// sign as this Constants' own `consumer` right now -- Ctx's own
// decision (`channels::brief::brief`'s own doc), not re-derived here.
// Deliberately not `brief.mine` or `brief.consumer_label`: both of
// those also count a vkh that's merely *labelled* (an independent
// Consumers entry, or a long-disconnected CIP-30 session saved for
// display) with no actual signing material behind it. Every action
// `line.can` can list is consumer-signed (`Will::signer`: `Sub`/
// `Settle` are the only provider-signed steps, and `consumer_can`
// already excludes both), so `can_sign` is the one condition that
// gates all four buttons. Staging a Want doesn't touch a wallet at all
// (`stageWant` only calls `basketPropose`); only `basket-submit` later
// needs one connected, and that's gated separately.
function renderLine(line, canSign) {
  const div = document.createElement("div");

  const described = describeCurrent(line.current, line.can);
  const sameFrontier = sameTxo(line.current?.txo_ref, line.finalized?.txo_ref);

  const summary = document.createElement("div");
  const finalText = line.finalized
    ? sameFrontier
      ? "final"
      : `final up to height ${line.finalized.created_at.height}`
    : "pending";
  summary.innerHTML = `
    <span class="${described.cls}">${described.text}</span>
    · <span class="${line.finalized ? "final" : "pending"}">${finalText}</span>
  `;
  div.appendChild(summary);

  const actions = document.createElement("div");
  if (line.can) {
    const { tx_hash, index } = line.current.txo_ref;
    actions.append(
      actionButton("Add", canSign && canAction(line.can, "Add"), () =>
        stageWant(tx_hash, index, { Add: { amount: 1000000 } }),
      ),
      actionButton("Close", canSign && canAction(line.can, "Close"), () =>
        stageWant(tx_hash, index, {
          Close: { upper: Date.now() + 5 * 60 * 1000 },
        }),
      ),
      actionButton("Elapse", canSign && canAction(line.can, "Elapse"), () =>
        stageWant(tx_hash, index, { Elapse: { lower: elapseAfter(line.can) } }),
      ),
      actionButton("End", canSign && canAction(line.can, "End"), () =>
        stageWant(tx_hash, index, "End"),
      ),
    );
  }
  div.appendChild(actions);

  // The unusual case: more than the two notable Txos above. Collapsed
  // by default -- the summary is the whole story in the typical case.
  const details = document.createElement("details");
  const summaryEl = document.createElement("summary");
  summaryEl.textContent = `Full history (${line.items.length} txo${line.items.length === 1 ? "" : "s"})`;
  details.appendChild(summaryEl);

  const table = document.createElement("table");
  table.innerHTML =
    "<thead><tr><th>height</th><th>txo_ref</th><th>stage</th><th>notes</th></tr></thead>";
  const tbody = document.createElement("tbody");
  for (const item of line.items) {
    const isCurrent = sameTxo(item.txo_ref, line.current?.txo_ref);
    const isFinalized = sameTxo(item.txo_ref, line.finalized?.txo_ref);
    const notes = [
      isFinalized ? "finalized" : null,
      isCurrent ? "current" : null,
      isCurrent && line.can ? `can: ${canLabel(line.can)}` : null,
    ]
      .filter(Boolean)
      .join(", ");
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td>${item.created_at.height}</td>
      <td>${item.txo_ref.tx_hash.slice(0, 8)}…#${item.txo_ref.index}</td>
      <td>${stagishLabel(item.stage)}</td>
      <td class="${isFinalized ? "final" : "pending"}">${notes || "—"}</td>
    `;
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  details.appendChild(table);
  div.appendChild(details);

  return div;
}

// One widget per `ConstantsLines`, index-dto's own per-Constants
// organization kept as-is (not re-grouped by keytag -- see
// `lineage.rs`'s own doc on why).
function renderConstantsWidget(constantsLines) {
  const c = constantsLines.constants;
  const brief = constantsLines.brief;
  const widget = document.createElement("div");
  widget.className = "channel-widget";

  const h3 = document.createElement("h3");
  const consumerText = brief.consumer_label ?? `${c.consumer.slice(0, 10)}…`;
  const providerText =
    brief.provider_matches[0]?.label ?? `${c.provider.slice(0, 10)}…`;
  h3.textContent = `${consumerText} → ${providerText}`;
  widget.appendChild(h3);
  widget.appendChild(renderBrief(brief, c));

  if (constantsLines.lines.length > 1) {
    const note = document.createElement("p");
    note.textContent = `${constantsLines.lines.length} parallel lines under this Constants (unusual -- distinct delegation partitions).`;
    widget.appendChild(note);
  }

  for (const line of constantsLines.lines) {
    widget.appendChild(renderLine(line, brief.can_sign));
  }

  return widget;
}

function renderLineage(view) {
  const container = el("lineage-out");
  container.innerHTML = "";

  if (view.channels.length === 0) {
    container.textContent = "(no channels found)";
    return;
  }
  for (const constantsLines of view.channels) {
    container.appendChild(renderConstantsWidget(constantsLines));
  }
}

// Recomputes lineage + "mine?"/available actions from the
// already-fetched history response (no Floe refetch) --
// `channelsLineageJson` reads Settings-tab state (Consumers/IouKeys/
// Keyring) and the live txo cache live on every call, so this is the
// Settings-tab-change -> Channels-tab-recolor link, cheap enough to
// call after every identity mutation, not just "Fetch from Floe". A
// no-op until the first fetch (`lastHistoryJson` still `null`).
function renderLineageFromCache() {
  if (!lastHistoryJson) return;
  const defaultDepth = Number(el("lineage-default-depth").value);
  const viewJson = store.channelsLineageJson(lastHistoryJson, defaultDepth);
  renderLineage(JSON.parse(viewJson));
}

el("lineage-fetch").addEventListener("click", async () => {
  const baseUrl = config.floe.baseUrl;
  if (!baseUrl) {
    log("fetch: no Floe base URL configured");
    activateTab("settings");
    return;
  }
  try {
    const validators = JSON.parse(store.whitelistJson());
    lastHistoryJson = await fetchHistory(baseUrl, validators);
    const liveItems = JSON.parse(lastHistoryJson).items.filter(
      (u) => u.spent_by == null,
    );
    store.basketRefreshTxosJson(JSON.stringify(liveItems));
    saveStore();
    renderLineageFromCache();
    log(`fetched: ${liveItems.length} live txo(s)`);
  } catch (e) {
    log(`fetch failed: ${e}`);
  }
});

// --- Settings tab: Keyring ---------------------------------------------------

function renderKeyring() {
  const labels = JSON.parse(store.keyringLabelsJson());
  renderItemList(
    el("keyring-out"),
    labels,
    (label) => label,
    (label) => {
      if (
        !confirm(
          `Remove keyring entry "${label}"? If you haven't backed up its key, it's gone for good.`,
        )
      )
        return;
      store.keyringRemove(label);
      saveStore();
      renderKeyring();
      renderLineageFromCache();
      log(`removed keyring entry ${label}`);
    },
  );
}
renderKeyring();

liveValidate(el("keyring-label"), () => validateNonEmpty(el("keyring-label")));
liveValidate(el("keyring-key"), () =>
  validateHex(el("keyring-key"), 32, { optional: true }),
);

el("keyring-add").addEventListener("click", () => {
  const labelInput = el("keyring-label");
  const keyInput = el("keyring-key");
  const labelOk = validateNonEmpty(labelInput);
  // Required here specifically -- "Add existing" has nothing to add
  // without a key, unlike "Generate new" sharing this same field.
  const keyOk = validateHex(keyInput, 32);
  if (!labelOk || !keyOk) return;
  try {
    store.keyringAdd(
      labelInput.value.trim(),
      keyInput.value.trim(),
      el("keyring-consumer").checked,
      el("keyring-iou").checked,
    );
    saveStore();
    labelInput.value = "";
    keyInput.value = "";
    renderKeyring();
    renderLineageFromCache();
  } catch (e) {
    log(`keyring add failed: ${e}`);
  }
});

el("keyring-generate").addEventListener("click", () => {
  const labelInput = el("keyring-label");
  if (!validateNonEmpty(labelInput)) return;
  try {
    const label = labelInput.value.trim();
    const keyHex = store.keyringGenerate(
      label,
      el("keyring-consumer").checked,
      el("keyring-iou").checked,
    );
    saveStore();
    labelInput.value = "";
    renderKeyring();
    renderLineageFromCache();
    log(
      `generated key under label ${label} (save this now, shown once):\n${keyHex}`,
    );
    drainLogs();
  } catch (e) {
    log(`keyring generate failed: ${e}`);
  }
});

// --- Settings tab: Consumers / IouKeys ---------------------------------------

// A Keyring entry flagged `is_consumer`/`is_iou` populates `effective`
// here too (see `Consumers`/`IouKeys`'s own doc), but `*Remove` only
// ever touches an *independent* entry -- removing one of those is a
// no-op Rust reports back as `false`, which is this file's cue to say
// why, rather than letting the button silently do nothing.
function renderConsumers() {
  const entries = JSON.parse(store.consumersEffectiveJson());
  renderItemList(
    el("consumers-out"),
    entries,
    ([label, vkh]) => `${label}: ${vkh}`,
    ([label]) => {
      if (!store.consumersRemove(label)) {
        log(
          `"${label}" comes from a Keyring entry -- remove or unflag it there instead`,
        );
        return;
      }
      saveStore();
      renderConsumers();
      renderLineageFromCache();
      log(`removed consumer ${label}`);
    },
  );
}
function renderIou() {
  const entries = JSON.parse(store.iouEffectiveJson());
  renderItemList(
    el("iou-out"),
    entries,
    ([label, key]) => `${label}: ${key}`,
    ([label]) => {
      if (!store.iouRemove(label)) {
        log(
          `"${label}" comes from a Keyring entry -- remove or unflag it there instead`,
        );
        return;
      }
      saveStore();
      renderIou();
      renderLineageFromCache();
      log(`removed iou_key ${label}`);
    },
  );
}
renderConsumers();
renderIou();

liveValidate(el("consumers-label"), () =>
  validateNonEmpty(el("consumers-label")),
);
liveValidate(el("consumers-vkh"), () => validateHex(el("consumers-vkh"), 28));

el("consumers-add").addEventListener("click", () => {
  const labelInput = el("consumers-label");
  const vkhInput = el("consumers-vkh");
  if (!validateNonEmpty(labelInput) || !validateHex(vkhInput, 28)) return;
  try {
    store.consumersAdd(labelInput.value.trim(), vkhInput.value.trim());
    saveStore();
    labelInput.value = "";
    vkhInput.value = "";
    renderConsumers();
    renderLineageFromCache();
  } catch (e) {
    log(`consumers add failed: ${e}`);
  }
});

liveValidate(el("iou-label"), () => validateNonEmpty(el("iou-label")));
liveValidate(el("iou-key"), () => validateHex(el("iou-key"), 32));

el("iou-add").addEventListener("click", () => {
  const labelInput = el("iou-label");
  const keyInput = el("iou-key");
  if (!validateNonEmpty(labelInput) || !validateHex(keyInput, 32)) return;
  try {
    store.iouAdd(labelInput.value.trim(), keyInput.value.trim());
    saveStore();
    labelInput.value = "";
    keyInput.value = "";
    renderIou();
    renderLineageFromCache();
  } catch (e) {
    log(`iou add failed: ${e}`);
  }
});

// --- Settings tab: Providers --------------------------------------------------

function renderProviders() {
  const entries = JSON.parse(store.providersJson());
  renderItemList(
    el("providers-out"),
    entries,
    ([label, entry]) =>
      `${label}: ${entry.base_url} (${entry.terms ? "terms known" : "no terms"})`,
    ([label]) => {
      store.providersRemove(label);
      saveStore();
      renderProviders();
      log(`removed provider ${label}`);
    },
  );
}
renderProviders();

liveValidate(el("providers-label"), () =>
  validateNonEmpty(el("providers-label")),
);
liveValidate(el("providers-base-url"), () =>
  validateUrl(el("providers-base-url")),
);

// The actual fetch (well-known path, GET, parse) lives in Rust now
// (`providersFetch` -- `ui::wasm`'s own GET against `subbit_l2_dto::
// WELL_KNOWN_PATH`, stamping `fetched_at` itself): this is just the
// button wiring.
el("providers-fetch").addEventListener("click", async () => {
  const labelInput = el("providers-label");
  const baseUrlInput = el("providers-base-url");
  if (!validateNonEmpty(labelInput) || !validateUrl(baseUrlInput)) return;
  try {
    await store.providersFetch(
      labelInput.value.trim(),
      baseUrlInput.value.trim(),
    );
    saveStore();
    renderProviders();
    drainLogs();
  } catch (e) {
    log(`providers fetch failed: ${e}`);
  }
});

el("providers-terms-asset-kind").addEventListener("change", () => {
  const isNative = el("providers-terms-asset-kind").value === "native";
  el("providers-terms-native-row").style.display = isNative ? "flex" : "none";
});

// Builds a full `Terms` for the "no discovery endpoint" case -- `null`
// if the provider vkh field is left blank (same "save the base_url,
// no Terms yet" case the button has always supported). The fields
// Console itself never acts on (`network`/`script`/`min_iou_amount`/
// `catalog`/`reference_script`) are filled with this build's own
// defaults rather than asked for here -- see the form's own note.
function buildManualTerms() {
  const providerInput = el("providers-terms-provider");
  const provider = providerInput.value.trim();
  if (provider === "") return null;
  if (!validateHex(providerInput, 28))
    throw new Error("provider vkh must be 28-byte hex");

  let asset;
  if (el("providers-terms-asset-kind").value === "native") {
    const hashInput = el("providers-terms-asset-hash");
    const nameInput = el("providers-terms-asset-name");
    if (!validateHex(hashInput, 28))
      throw new Error("asset policy id must be 28-byte hex");
    const name = nameInput.value.trim();
    if (name === "" || !/^[0-9a-fA-F]+$/.test(name))
      throw new Error("asset name must be hex");
    asset = { Native: { hash: hashInput.value.trim(), name } };
  } else {
    asset = "Lovelace";
  }

  const closePeriodSecs = Number(el("providers-terms-close-period").value);
  if (!Number.isFinite(closePeriodSecs) || closePeriodSecs <= 0) {
    throw new Error("close period must be a positive number of seconds");
  }
  const confirmationDepth = Number(
    el("providers-terms-confirmation-depth").value,
  );
  if (!Number.isInteger(confirmationDepth) || confirmationDepth < 0) {
    throw new Error("confirmation depth must be a non-negative integer");
  }

  return {
    network: config.floe.network === "mainnet" ? "Mainnet" : "Preprod",
    script: defaultValidatorHash(),
    asset,
    provider,
    close_period: closePeriodSecs * 1000,
    confirmation_depth: confirmationDepth,
    min_iou_amount: 0,
    catalog: {
      default: {
        cost: { ByRequest: 0 },
        description: "",
        mime_type: "",
        max_timeout_seconds: 0,
        service_name: null,
        tags: null,
        icon_url: null,
      },
      resources: [],
    },
    reference_script: null,
  };
}

el("providers-save-manual").addEventListener("click", () => {
  const labelInput = el("providers-label");
  const baseUrlInput = el("providers-base-url");
  if (!validateNonEmpty(labelInput) || !validateUrl(baseUrlInput)) return;
  try {
    const terms = buildManualTerms();
    const fetchedAt = terms ? Math.round(Date.now() / 1000) : undefined;
    store.providersSet(
      labelInput.value.trim(),
      baseUrlInput.value.trim(),
      terms ? JSON.stringify(terms) : undefined,
      fetchedAt,
    );
    saveStore();
    renderProviders();
    drainLogs();
  } catch (e) {
    log(`providers save failed: ${e}`);
  }
});

// --- Tx tab: open-channel form ------------------------------------------------

el("open-generate-tag").addEventListener("click", () => {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  el("open-tag").value = [...bytes]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
});

el("open-stage").addEventListener("click", () => {
  try {
    const providerLabel = el("open-provider-label").value.trim();
    const providers = Object.fromEntries(JSON.parse(store.providersJson()));
    const entry = providers[providerLabel];
    const terms = entry?.terms;

    const iouLabel = el("open-iou-label").value.trim();
    const iouKeys = Object.fromEntries(JSON.parse(store.iouEffectiveJson()));
    const iouKey = iouKeys[iouLabel];
    if (!iouKey) throw new Error(`no iou_key known under label ${iouLabel}`);
    if (!wallet) throw new Error("connect a wallet first (consumer vkh)");

    const tag = el("open-tag").value.trim();
    const assetJson = terms
      ? JSON.stringify(terms.asset)
      : JSON.stringify("Lovelace");
    const provider = terms
      ? terms.provider
      : (prompt("manual mode: provider vkh (hex)?") ?? "");
    const closePeriodSecs = terms
      ? Math.round(terms.close_period / 1000)
      : Number(el("open-close-period").value);
    const amount = Number(el("open-amount").value);

    store.basketAddOpen(
      tag,
      assetJson,
      iouKey,
      wallet.vkh,
      provider,
      BigInt(closePeriodSecs),
      undefined,
      BigInt(amount),
    );
    saveStore();
    renderBasket();
    el("open-out").textContent = JSON.stringify({
      tag,
      iouKey,
      consumer: wallet.vkh,
      provider,
      closePeriodSecs,
      amount,
    });
    drainLogs();
  } catch (e) {
    log(`open stage failed: ${e}`);
  }
});

// --- Tx tab: basket panel + submit -------------------------------------------

function updateTxBadge() {
  const opens = JSON.parse(store.basketOpensJson());
  const wills = JSON.parse(store.basketWillsJson());
  const count = Object.keys(opens).length + wills.length;
  el("tx-badge").textContent = count > 0 ? `(${count})` : "";
}

function renderBasket() {
  el("basket-opens-out").textContent = store.basketOpensJson();
  el("basket-wills-out").textContent = store.basketWillsJson();
  updateTxBadge();
}
renderBasket();

el("basket-drop-all-opens").addEventListener("click", () => {
  store.basketDropAllOpens();
  saveStore();
  renderBasket();
});

el("basket-drop-all-intents").addEventListener("click", () => {
  store.basketDropAllIntents();
  saveStore();
  renderBasket();
});

// Prefers a provider's own published reference script (`Terms.
// reference_script`, see `Ctx::basket_set_reference_utxo_from_terms`'s
// own doc) over self-hosting one -- tries every known provider's terms
// in turn (first live match wins), falling back to whatever reference
// script this wallet itself holds (`findReferenceScriptJson`, the
// pre-existing self-upload path) only if no provider's own checks out.
// Returns `"provider"` / `"self-hosted"` / `null` (nothing resolved).
function resolveReferenceUtxo(liveItemsJson) {
  const providers = JSON.parse(store.providersJson()); // [[label, entry], ...]
  for (const [, entry] of providers) {
    if (!entry.terms || !entry.terms.reference_script) continue;
    try {
      store.basketSetReferenceUtxoFromTerms(
        JSON.stringify(entry.terms),
        liveItemsJson,
      );
      return "provider";
    } catch (e) {
      // This provider's published reference script isn't live/found
      // among `liveItemsJson` -- try the next provider.
    }
  }
  const referenceUtxoJson = findReferenceScriptJson(liveItemsJson);
  if (referenceUtxoJson) {
    store.basketSetReferenceUtxo(referenceUtxoJson);
    return "self-hosted";
  }
  return null;
}

el("basket-submit").addEventListener("click", async () => {
  if (!submitBackend) {
    log("submit: no wallet connected");
    return;
  }
  const baseUrl = config.floe.baseUrl;
  try {
    const validators = JSON.parse(store.whitelistJson());
    // A fresh fetch, not `lastHistoryJson` -- building a real spending
    // tx needs a current snapshot, not whatever was live whenever
    // Lineage was last fetched.
    const historyJson = await fetchHistory(baseUrl, validators);
    const liveItems = JSON.parse(historyJson).items.filter(
      (u) => u.spent_by == null,
    );
    const resolvedFrom = resolveReferenceUtxo(JSON.stringify(liveItems));
    if (!resolvedFrom) {
      log(
        "submit: no reference script set -- upload one (Settings tab) or add a provider that publishes one",
      );
      el("basket-submit-out").textContent = "error: no reference script set";
      return;
    }
    log(`submit: using ${resolvedFrom} reference script`);

    // Build, sign, attach the witness, and submit all happen in Rust
    // now (`ui::wasm::submit`), against whichever backend
    // `connectCip30`/`connectEmbedded` last connected -- this file no
    // longer calls `wallet.api.*` (or anything `Embedded`-specific)
    // directly.
    const txHash = await store.submit(config.floe.network);

    saveStore();
    renderBasket();
    el("basket-submit-out").textContent = `submitted: ${txHash}`;
    log(`submit succeeded, tx=${txHash}`);
  } catch (e) {
    log(`submit failed: ${e}`);
    el("basket-submit-out").textContent = `error: ${e}`;
  }
});

// --- Settings tab: embedded key management -------------------------------------
//
// This section manages the key/delegation Console's own Store holds.
// Uses Floe's own configured network (`config.floe.network`), same as
// submit -- there's no separate network setting to drift from it.
// "Use for submit" (below) is the one action that actually connects
// it as this session's submit wallet -- everything else here just
// manages the key material.

function renderEmbeddedWallet() {
  const view = JSON.parse(store.walletViewJson());
  if (!view.has_key) {
    el("wallet-out").textContent = "(no key configured)";
    return;
  }
  let address = "(error deriving address)";
  try {
    address = store.walletAddress(config.floe.network) ?? "(none)";
  } catch (e) {
    address = `(error: ${e})`;
  }
  el("wallet-out").textContent =
    `key: configured\ndelegation: ${view.has_delegation ? "set" : "none"}\n` +
    `address (${config.floe.network}): ${address}`;
}
renderEmbeddedWallet();

el("wallet-use-for-submit").addEventListener("click", async () => {
  const baseUrl = config.floe.baseUrl;
  if (!baseUrl) {
    log("use for submit: no Floe base URL configured");
    activateTab("settings");
    return;
  }
  try {
    await store.connectEmbedded(baseUrl, config.floe.network);
    // This session's one submit slot just moved to Embedded -- drop
    // the CIP-30 connection's own display state so it doesn't claim to
    // still be the active one.
    wallet = null;
    submitBackend = "embedded";
    el("wallet-connection-out").textContent = "connected: embedded wallet";
    el("basket-submit").disabled = false;
    log("embedded wallet connected for submit");
  } catch (e) {
    log(`use for submit failed: ${e}`);
  }
});

el("wallet-generate").addEventListener("click", () => {
  const view = JSON.parse(store.walletViewJson());
  if (
    view.has_key &&
    !confirm(
      "Replace the existing embedded key? Back it up first if you haven't.",
    )
  )
    return;
  try {
    const hex = store.walletGenerate();
    saveStore();
    renderEmbeddedWallet();
    log(`generated embedded wallet key (save this now, shown once):\n${hex}`);
  } catch (e) {
    log(`wallet generate failed: ${e}`);
  }
});

liveValidate(el("wallet-import-hex"), () =>
  validateHex(el("wallet-import-hex"), 32),
);

el("wallet-import").addEventListener("click", () => {
  const keyInput = el("wallet-import-hex");
  if (!validateHex(keyInput, 32)) return;
  try {
    store.walletImport(keyInput.value.trim());
    saveStore();
    keyInput.value = "";
    renderEmbeddedWallet();
    drainLogs();
  } catch (e) {
    log(`wallet import failed: ${e}`);
  }
});

liveValidate(el("wallet-delegation-hex"), () =>
  validateHex(el("wallet-delegation-hex"), 28),
);

el("wallet-set-delegation").addEventListener("click", () => {
  const delegationInput = el("wallet-delegation-hex");
  if (!validateHex(delegationInput, 28)) return;
  try {
    store.walletSetDelegation(delegationInput.value.trim());
    saveStore();
    renderEmbeddedWallet();
  } catch (e) {
    log(`wallet set delegation failed: ${e}`);
  }
});

el("wallet-clear-delegation").addEventListener("click", () => {
  store.walletClearDelegation();
  saveStore();
  renderEmbeddedWallet();
});

el("wallet-reveal").addEventListener("click", () => {
  const hex = store.walletReveal();
  if (hex == null) {
    drainLogs();
    return;
  }
  log(`embedded wallet key:\n${hex}`);
});

el("wallet-export").addEventListener("click", () => {
  try {
    const json = store.walletExportJson();
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "subbit-console-wallet-export.json";
    a.click();
    URL.revokeObjectURL(url);
  } catch (e) {
    log(`wallet export failed: ${e}`);
  }
});

el("wallet-scrub").addEventListener("click", () => {
  if (
    !confirm(
      "Scrub the embedded wallet? This deletes the key and delegation. This cannot be undone.",
    )
  )
    return;
  store.walletScrub();
  // `connectEmbedded` already copied the signing key into this
  // session's submit slot -- scrubbing the stored key wouldn't
  // otherwise disconnect it.
  if (submitBackend === "embedded") {
    store.disconnectWallet();
    submitBackend = null;
    el("wallet-connection-out").textContent = "(not connected)";
    el("basket-submit").disabled = true;
  }
  saveStore();
  renderEmbeddedWallet();
  drainLogs();
});

// --- Settings tab: reference script (upload/teardown) -----------------------
//
// Acts on whichever wallet `wallet-use-for-submit`/`connectCip30` last
// connected (`submitBackend`), same as `basket-submit` -- neither of
// these touches the basket's own reference-script setting
// (`basketSetReferenceUtxo`/`FromTerms`, wired into `basket-submit`'s
// own handler above), they only create/reclaim the on-chain utxo
// itself.

el("reference-script-upload").addEventListener("click", async () => {
  if (!submitBackend) {
    log("upload reference script: no wallet connected");
    return;
  }
  const addressInput = el("reference-script-address").value.trim();
  try {
    const hash = await store.uploadReferenceScript(
      config.floe.network,
      addressInput === "" ? undefined : addressInput,
    );
    el("reference-script-out").textContent = `uploaded: ${hash}`;
    log(`reference script uploaded, tx=${hash}`);
  } catch (e) {
    log(`upload reference script failed: ${e}`);
    el("reference-script-out").textContent = `error: ${e}`;
  }
});

el("reference-script-teardown").addEventListener("click", async () => {
  if (!submitBackend) {
    log("teardown reference script: no wallet connected");
    return;
  }
  if (
    !confirm(
      "Tear down this wallet's held reference script? It won't be usable by anything still relying on it.",
    )
  )
    return;
  try {
    const hash = await store.teardownReferenceScript(config.floe.network);
    el("reference-script-out").textContent = `torn down: ${hash}`;
    log(`reference script torn down, tx=${hash}`);
  } catch (e) {
    log(`teardown reference script failed: ${e}`);
    el("reference-script-out").textContent = `error: ${e}`;
  }
});

// --- Settings tab: data management (a destructive, whole-store setting) -----

el("data-export").addEventListener("click", () => {
  const blob = new Blob([store.toJson()], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "subbit-console-export.json";
  a.click();
  URL.revokeObjectURL(url);
});

el("data-import").addEventListener("change", async (ev) => {
  const file = ev.target.files[0];
  if (!file) return;
  try {
    const text = await file.text();
    store = new JsConsole(text); // validates before replacing the live store
    saveStore();
    renderAll();
    log("import succeeded");
  } catch (e) {
    log(`import failed (store left unchanged): ${e}`);
  }
});

el("data-delete").addEventListener("click", () => {
  if (!confirm("Delete data? This cannot be undone.")) return;
  store = new JsConsole(undefined);
  store.save(); // overwrites the persisted blob with this fresh default
  renderAll();
  log("all data deleted");
});

function renderAll() {
  renderWhitelist();
  renderKeyring();
  renderConsumers();
  renderIou();
  renderProviders();
  renderBasket();
  renderEmbeddedWallet();
  lastHistoryJson = null;
  el("lineage-out").textContent = "(not fetched)";
}

// No Floe backend configured yet -- focus Settings instead of an empty
// Channels tab with nothing to show and nothing to fetch.
if (!config.floe.baseUrl) {
  activateTab("settings");
  el("floe-base-url").focus();
}

log("ready");
