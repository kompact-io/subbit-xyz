//! Shared cost-resolution engine. `dto` holds the wire shapes
//! (`Cost`/`CostByEgress`/`ResourceInfo`/`Catalog`) that actually ride
//! over the network, e.g. as `subbit_l2_dto::Terms.catalog`. The
//! top-level `Catalog` here is the compiled, matchit-backed resolver
//! built from `dto::Catalog` -- identical logic for a provider
//! (deciding what to charge) and a consumer (deciding what it expects
//! to pay). The two share the name `Catalog`, disambiguated only by
//! module path, on purpose: `dto::Catalog` is inert data, `Catalog` is
//! the thing you actually resolve a path against.
//!
//! What happens when nothing matches -- a provider's own blanket
//! policy, a consumer seeking a caller's consent -- is deliberately
//! not represented here: that's a caller problem, not a shared trait
//! or a generic parameter on `Catalog`.

pub mod dto;

mod catalog;
pub use catalog::{Catalog, CatalogError};
