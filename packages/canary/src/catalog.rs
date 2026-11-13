//! Canary's own fixed demo catalog: one `roman/<numeral>` route per classical Roman
//! numeral symbol, each costing `unit * that symbol's own value`. A stand-in for a
//! provider's published catalog until this becomes part of the API -- for now,
//! both `Provider` (building gate's `Terms.catalog`) and `consumer::agent::Agent`
//! (picking which route its FSM targets) read this one shared table; neither side
//! invents a cost independently -- the authoritative per-request price lives
//! only in the catalog itself (resolved identically by `subbit_stamp` client-side and
//! `subbit_gate` server-side), never echoed or guessed by canary's own code.
//!
//! Registered twice per level, under both path shapes the system actually uses:
//! gate's own internal lookup strips the `/v1/spend` prefix before matching
//! (`packages/gate/src/app.rs`'s `run_spend_blocking`: `format!("/{path}")`), while
//! `subbit_stamp`'s client middleware matches against the full request path it's about
//! to send (`req.url().path()`, e.g. `/v1/spend/roman/i`) -- so a route only resolves
//! correctly on both sides if both shapes are in the catalog. The previous single
//! `"/{*rest}"` catch-all dodged this by matching literally everything; a per-tier
//! catalog can't.

use subbit_l2_dto::{Catalog, Cost, ResourceInfo};

/// Every level this catalog knows, lowest value first.
pub const LEVELS: &[char] = &['i', 'v', 'x', 'l', 'c', 'd', 'm'];

/// The classical Roman numeral value of one `LEVELS` symbol. `0` for anything else.
pub fn value(level: char) -> u64 {
    match level {
        'i' => 1,
        'v' => 5,
        'x' => 10,
        'l' => 50,
        'c' => 100,
        'd' => 500,
        'm' => 1000,
        _ => 0,
    }
}

/// `level`'s own route, relative to `/v1/spend/` -- e.g. `roman/i`.
pub fn path(level: char) -> String {
    format!("roman/{level}")
}

/// The cost catalog both `Provider` and every `Agent` read -- one `ByRequest`
/// route per `LEVELS` symbol, each `unit * value(level)`. See this module's own doc
/// comment on why each level is registered under two path shapes.
pub fn dto(unit: u64) -> Catalog {
    let resources = LEVELS
        .iter()
        .flat_map(|&level| {
            let info = || ResourceInfo {
                cost: Cost::ByRequest(unit * value(level)),
                description: format!("Roman-numeral tier {level}"),
                mime_type: "text/plain".to_string(),
                max_timeout_seconds: 60,
                service_name: None,
                tags: None,
                icon_url: None,
            };
            [
                (format!("/{}", path(level)), info()),
                (format!("/v1/spend/{}", path(level)), info()),
            ]
        })
        .collect();
    Catalog { resources }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the exact same `subbit_cost::Catalog` (matchit-backed) matcher both
    /// `subbit_gate::Ctx::cost` (server-side) and `subbit_stamp::Stamp::resolve_charge`
    /// (client-side) actually use -- confirming both path shapes this module's own doc
    /// comment describes really do resolve, and to the right, scaled cost.
    #[test]
    fn both_path_shapes_resolve_to_the_same_scaled_cost() {
        let compiled = subbit_cost::Catalog::new(dto(100)).expect("every pattern is literal");
        assert_eq!(compiled.cost("/roman/i"), Some(100)); // server-side shape
        assert_eq!(compiled.cost("/v1/spend/roman/i"), Some(100)); // client-side shape
        assert_eq!(compiled.cost("/roman/x"), Some(1_000));
        assert_eq!(compiled.cost("/v1/spend/roman/x"), Some(1_000));
        assert_eq!(compiled.cost("/roman/m"), Some(100_000));
    }

    #[test]
    fn an_unknown_level_is_out_of_catalog_not_a_silent_zero() {
        let compiled = subbit_cost::Catalog::new(dto(100)).unwrap();
        assert_eq!(compiled.cost("/roman/z"), None);
    }
}
