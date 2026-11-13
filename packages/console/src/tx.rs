//! Tx tab: building -- staging an Open/Want and checking out.
//! "History"/lineage display is a Channels-tab concern instead (see
//! `channels::lineage`'s own doc): Tx is purely the acting side,
//! Channels is purely the observing side (live snapshot *and* full
//! lineage, both are "what do I see"). The actual staging/pick/build
//! state lives in `subbit_tx::queue::Queue` directly now -- no console-
//! level wrapper (`Basket` is gone; it was pure delegation). `Ctx`'s
//! own `basket_*`/`basketXxx` method names stay as they are (not
//! renamed to `queue_*`) to avoid a wide, low-value rename across
//! every already-shipped identifier in `wasm.rs`/`main.rs`/`app.js`.

mod submit;
pub use submit::submit;

mod plain;
pub use plain::{send_payment, sweep, teardown_reference_script, upload_reference_script};
