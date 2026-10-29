//! Core domain logic behind `subbit-console`: everything the Settings
//! tab renders (`settings`: identity bookkeeping -- Keyring,
//! Consumers, IouKeys -- plus Providers, the whitelisted-validator
//! list, Floe I/O glue, and the embedded wallet's own key/delegation
//! state), the basket (`Basket`, wrapping `subbit_tx::tx::Tx` directly
//! -- a Rust-level dependency, not just JSON at a wasm boundary, so
//! this is one unified core rather than tx-building logic living only
//! inside a browser-only wasm feature), and `logs` (a small, shared
//! action-narration log -- see its own doc for why it isn't part of
//! `Ctx`).
//!
//! `Ctx` is the single application model every UI sits on -- there is
//! no separate `Store` type any more (deleted: its fields live
//! directly on `Ctx`, which derives `Serialize`/`Deserialize` itself,
//! nothing more: no target-gated code, no persistence methods).
//! `Ctx`'s own public signatures *are* the API, not a separately-
//! curated type list (an earlier `api` module just re-exported them a
//! second time, with no content of its own; this module's own
//! re-exports already cover it). Frontend-agnostic on purpose
//! (plan-subbit-console.md: "should work both as a standalone app, and
//! as a lib usable by other rust libs or other frontends") -- no
//! `wasm_bindgen` types appear here; those, and *where*/*how* to
//! persist, live only in `ui` (`ui::wasm`, gated by `target_arch`, not
//! a Cargo feature -- see `ui.rs`'s own doc; `ui::cli`, gated by the
//! `cli` feature) -- each a thin adapter that
//! should be boilerplate: parse args, call a `Ctx` method, serialize
//! the result, pick a storage location. `main.rs` is a one-line shim
//! calling `ui::cli::run`; a future native TUI ("an interface very
//! similar to the web-app") has the same shape to reuse.

mod error;
pub use error::Error;

mod settings;
pub use settings::{
    Consumers, IouKeys, Keyring, ProviderEntry, Providers, SigningKeyHex, WalletView, Whitelist,
    discover_history,
};

mod channels;
pub use channels::{
    Brief, ConstantsLines, Divergence, LineView, LineageView, Match, ProviderMatch, Status,
    is_final, reconcile, statuses,
};

mod tx;
pub use tx::Basket;

mod logs;
pub use logs::{Entry, Level, Logs};

mod ctx;
pub use ctx::Ctx;

pub mod ui;
