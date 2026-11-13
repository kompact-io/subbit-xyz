//! Finality annotation for the dashboard's "lines" view
//! (plan-subbit-console.md section J): whether a lineage item has
//! reached a matched Terms' own `confirmation_depth` yet. "Supposedly"
//! final, not actually final -- this is a depth heuristic Console
//! computes itself from `subbit_index_dto::Point::height`s, not a
//! protocol guarantee. Frontend-agnostic core (no `wasm_bindgen`), same
//! as the rest of this crate -- `wasm::is_final` just forwards to it.

/// `tip_height` and `item_height` are both `subbit_index_dto::Point::
/// height` (the reconstructed lineage's own chain-tip and per-item
/// creation height -- see `subbit_index::lineage::reconstruct`'s
/// `Output`). `confirmation_depth` is the matched `Terms`' own field.
pub fn is_final(tip_height: u64, item_height: u64, confirmation_depth: u32) -> bool {
    tip_height.saturating_sub(item_height) >= u64::from(confirmation_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_at_depth_is_final() {
        assert!(is_final(10, 7, 3));
    }

    #[test]
    fn short_of_depth_is_not_final() {
        assert!(!is_final(10, 8, 3));
    }

    #[test]
    fn item_newer_than_tip_does_not_underflow() {
        // Shouldn't happen in practice, but a saturating sub keeps this
        // from panicking if a stale/inconsistent tip is ever handed in.
        assert!(!is_final(5, 10, 1));
    }
}
