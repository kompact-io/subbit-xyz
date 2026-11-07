//! Stamp generates and manages IOUs client side.

pub mod time;
pub use time::now;

pub mod id;
pub use id::Id;

pub mod config;
pub use config::Config;

pub mod cache;
pub use cache::Cache;

pub mod catalog;
pub use catalog::{Catalog, CatalogError, Charge, MissDecision, MissPolicy};

pub mod stamp;
pub use stamp::{ResponseError, Stamp};

pub mod dialect;

#[cfg(feature = "reqwest")]
pub mod reqwest;

#[cfg(feature = "local")]
pub mod local;

#[cfg(feature = "wasm")]
pub mod wasm;
