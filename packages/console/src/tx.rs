//! Tx tab: building -- staging an Open/Want into the basket and
//! checking out. "History"/lineage display is a Channels-tab concern
//! instead (see `channels::lineage`'s own doc): Tx is purely the
//! acting side, Channels is purely the observing side (live snapshot
//! *and* full lineage, both are "what do I see"). `basket` stays a
//! submodule here (not renamed to `Tx`, to avoid a wide, low-value
//! rename across every already-shipped `basket_*`/`basketXxx`
//! identifier in `Ctx`/`wasm.rs`/`main.rs`/`app.js`) -- the module
//! boundary moves to reflect the Tx tab; the type name doesn't need to.

pub mod basket;
pub use basket::Basket;
