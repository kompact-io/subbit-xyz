//! Joins an observed txo's `Constants` against whatever
//! `Providers`'s `by_provider` lookup found, and reports *divergence* --
//! not just a boolean match/no-match. Deliberately its own module, not a
//! method on `Providers` or `Constants`: `Providers` is a pure store
//! (label/base_url/terms/fetched_at CRUD + the by-provider query), and
//! should stay that way; this is where declared data (`Terms`) and
//! observed data (`Constants`, from `subbit-l1-dto`, not `subbit-tx` --
//! same "depend on the pure DTO, not the tx-building crate" boundary
//! this crate's own `floe.rs` uses) get compared.
//!
//! Mirrors `subbit-index`'s `naive.rs` precedent: a declared-config type
//! (`naive::Config`) and an observed type are kept separate and joined
//! only through an explicit matcher function (`is_backing`), rather than
//! merged into one struct. The difference here: `is_backing` is a
//! boolean filter (mismatches are dropped); this reports *which* fields
//! diverge, since a UI needs to say "matches ACME's key, but the
//! close_period on this txo doesn't meet their terms" rather than
//! just excluding it silently.

use subbit_l1_dto::Constants;

use crate::ProviderEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Divergence {
    /// The txo's `asset` doesn't match `Terms.asset` at all --
    /// exact equality, there's no "close enough" for an asset class.
    Asset,
    /// The txo's `close_period` is shorter than `Terms.close_period`
    /// requires. A *longer* close_period than required is fine (mirrors
    /// `subbit_index::naive::is_backing`'s own `>=` check, not `==`) --
    /// only "too short" is a problem.
    ClosePeriodTooShort,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Match<'a> {
    pub label: &'a str,
    pub entry: &'a ProviderEntry,
    /// Empty iff this entry's `Terms` fully agrees with the txo's
    /// own constants (or the entry has no cached `Terms` yet to compare
    /// against, in which case nothing can be said to diverge).
    pub divergences: Vec<Divergence>,
}

/// `matches` is expected to be `Providers::by_provider`'s own output
/// (every entry whose `Terms.provider` equals `constants.provider()`) --
/// this function doesn't re-check the provider key itself, only the
/// remaining fields, on whatever's handed to it.
pub fn reconcile<'a>(
    constants: &Constants,
    matches: Vec<(&'a String, &'a ProviderEntry)>,
) -> Vec<Match<'a>> {
    matches
        .into_iter()
        .map(|(label, entry)| {
            let mut divergences = Vec::new();
            if let Some(terms) = &entry.terms {
                if &terms.asset != constants.asset() {
                    divergences.push(Divergence::Asset);
                }
                if constants.close_period() < &terms.close_period {
                    divergences.push(Divergence::ClosePeriodTooShort);
                }
            }
            Match {
                label,
                entry,
                divergences,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Providers;
    use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
    use subbit_l1_dto::Asset;
    use subbit_l2_dto::{Catalog, Cost, Network, ResourceInfo, Terms};

    fn constants(asset: Asset, close_period: Duration, provider: Hash28) -> Constants {
        Constants::new(
            Tag::from(b"t".to_vec()),
            asset,
            VerifyingKey::from([0u8; 32]),
            Hash28::from([0u8; 28]),
            provider,
            close_period,
        )
    }

    fn terms(asset: Asset, close_period: Duration, provider: Hash28) -> Terms {
        Terms {
            network: Network::Preprod,
            script: Hash28::from([0u8; 28]),
            asset,
            provider,
            close_period,
            confirmation_depth: 3,
            min_iou_amount: 0,
            catalog: Catalog {
                resources: vec![(
                    "/{*rest}".to_string(),
                    ResourceInfo {
                        cost: Cost::ByRequest(0),
                        description: String::new(),
                        mime_type: String::new(),
                        max_timeout_seconds: 0,
                        service_name: None,
                        tags: None,
                        icon_url: None,
                    },
                )],
            },
            reference_script: None,
        }
    }

    #[test]
    fn exact_agreement_has_no_divergence() {
        let provider = Hash28::from([9u8; 28]);
        let period = Duration::from_secs(900);
        let c = constants(Asset::Lovelace, period, provider);

        let mut book = Providers::default();
        book.insert(
            "acme".to_string(),
            ProviderEntry::manual(
                "https://acme.example".to_string(),
                terms(Asset::Lovelace, period, provider),
            ),
        )
        .unwrap();

        let matches = book.by_provider(&provider);
        let results = reconcile(&c, matches);
        assert_eq!(results.len(), 1);
        assert!(results[0].divergences.is_empty());
    }

    #[test]
    fn longer_close_period_than_required_is_not_a_divergence() {
        let provider = Hash28::from([9u8; 28]);
        let required = Duration::from_secs(900);
        let actual = Duration::from_secs(1800); // longer, i.e. safer
        let c = constants(Asset::Lovelace, actual, provider);

        let mut book = Providers::default();
        book.insert(
            "acme".to_string(),
            ProviderEntry::manual(
                "https://acme.example".to_string(),
                terms(Asset::Lovelace, required, provider),
            ),
        )
        .unwrap();

        let results = reconcile(&c, book.by_provider(&provider));
        assert!(results[0].divergences.is_empty());
    }

    #[test]
    fn shorter_close_period_and_wrong_asset_are_both_flagged() {
        let provider = Hash28::from([9u8; 28]);
        let required = Duration::from_secs(1800);
        let actual = Duration::from_secs(900); // shorter: too risky
        let c = constants(Asset::Lovelace, actual, provider);
        let wrong_asset = Asset::Native {
            hash: Hash28::from([3u8; 28]),
            name: b"tok".to_vec(),
        };

        let mut book = Providers::default();
        book.insert(
            "acme".to_string(),
            ProviderEntry::manual(
                "https://acme.example".to_string(),
                terms(wrong_asset, required, provider),
            ),
        )
        .unwrap();

        let results = reconcile(&c, book.by_provider(&provider));
        assert_eq!(
            results[0].divergences,
            vec![Divergence::Asset, Divergence::ClosePeriodTooShort]
        );
    }

    #[test]
    fn multiple_services_on_one_key_are_each_reconciled_independently() {
        let provider = Hash28::from([9u8; 28]);
        let period = Duration::from_secs(900);
        let c = constants(Asset::Lovelace, period, provider);

        let mut book = Providers::default();
        book.insert(
            "matches".to_string(),
            ProviderEntry::manual(
                "https://ok.example".to_string(),
                terms(Asset::Lovelace, period, provider),
            ),
        )
        .unwrap();
        book.insert(
            "diverges".to_string(),
            ProviderEntry::manual(
                "https://strict.example".to_string(),
                terms(Asset::Lovelace, Duration::from_secs(3600), provider),
            ),
        )
        .unwrap();

        let mut results = reconcile(&c, book.by_provider(&provider));
        results.sort_by_key(|m| m.label.to_string());

        assert!(
            results[0]
                .divergences
                .contains(&Divergence::ClosePeriodTooShort)
        );
        assert!(results[1].divergences.is_empty());
    }
}
