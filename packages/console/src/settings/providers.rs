//! A labelled store of known providers (plan-subbit-console.md section
//! F): base_url + a cached `Terms`, re-fetchable on demand (fetching
//! itself is the caller's job -- this module only stores the result),
//! and freely editable even after a fetch (`fetched_at` is kept so a
//! frontend can flag a field that has since drifted from what was last
//! fetched, but nothing here enforces that -- see section F's
//! "pre-fills, doesn't lock" resolution).

use std::collections::BTreeMap;

use subbit_core::Hash28;
use subbit_l2_dto::Terms;

use crate::Error;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderEntry {
    pub base_url: String,
    pub terms: Option<Terms>,
    /// Unix seconds of the last successful fetch; `None` for an entry
    /// that was always hand-entered.
    pub fetched_at: Option<u64>,
}

impl ProviderEntry {
    pub fn manual(base_url: String, terms: Terms) -> Self {
        Self {
            base_url,
            terms: Some(terms),
            fetched_at: None,
        }
    }

    pub fn fetched(base_url: String, terms: Terms, fetched_at: u64) -> Self {
        Self {
            base_url,
            terms: Some(terms),
            fetched_at: Some(fetched_at),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Providers {
    entries: BTreeMap<String, ProviderEntry>,
}

impl Providers {
    pub fn insert(&mut self, label: String, entry: ProviderEntry) -> Result<(), Error> {
        if self.entries.contains_key(&label) {
            return Err(Error::LabelTaken(label));
        }
        self.entries.insert(label, entry);
        Ok(())
    }

    /// Replaces an existing entry (re-fetch, or a manual edit) -- unlike
    /// `insert`, this is expected to overwrite.
    pub fn set(&mut self, label: String, entry: ProviderEntry) {
        self.entries.insert(label, entry);
    }

    pub fn remove(&mut self, label: &str) -> Option<ProviderEntry> {
        self.entries.remove(label)
    }

    pub fn get(&self, label: &str) -> Option<&ProviderEntry> {
        self.entries.get(label)
    }

    pub fn labels(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ProviderEntry)> {
        self.entries.iter()
    }

    /// Every entry whose cached `Terms.provider` matches `provider` --
    /// the lookup a discovered on-chain txo's own `Constants.
    /// provider` vkh drives, to give it a label/context. Deliberately
    /// not a single `Option`: one provider key can legitimately back
    /// more than one labelled service (distinct `Terms`, e.g. different
    /// catalogs or `close_period`s under the same operator key), so a
    /// txo's provider key can match several entries here at once --
    /// this is a many-match query, not a unique lookup.
    pub fn by_provider(&self, provider: &Hash28) -> Vec<(&String, &ProviderEntry)> {
        self.entries
            .iter()
            .filter(|(_, entry)| {
                entry
                    .terms
                    .as_ref()
                    .is_some_and(|terms| &terms.provider == provider)
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use subbit_core::Duration;
    use subbit_l2_dto::{Asset, Catalog, Cost, Network, ResourceInfo};

    fn terms(provider: Hash28) -> Terms {
        Terms {
            network: Network::Preprod,
            script: Hash28::from([0u8; 28]),
            asset: Asset::Lovelace,
            provider,
            close_period: Duration::from_secs(900),
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
    fn by_provider_returns_every_service_sharing_that_key() {
        let shared = Hash28::from([9u8; 28]);
        let mut book = Providers::default();
        book.insert(
            "metered-api".to_string(),
            ProviderEntry::manual("https://metered.example".to_string(), terms(shared)),
        )
        .unwrap();
        book.insert(
            "bulk-api".to_string(),
            ProviderEntry::manual("https://bulk.example".to_string(), terms(shared)),
        )
        .unwrap();
        book.insert(
            "unrelated".to_string(),
            ProviderEntry::manual(
                "https://unrelated.example".to_string(),
                terms(Hash28::from([1u8; 28])),
            ),
        )
        .unwrap();

        let mut labels: Vec<_> = book
            .by_provider(&shared)
            .into_iter()
            .map(|(label, _)| label.clone())
            .collect();
        labels.sort();
        assert_eq!(
            labels,
            vec!["bulk-api".to_string(), "metered-api".to_string()]
        );
    }

    #[test]
    fn by_provider_skips_entries_with_no_terms() {
        let mut book = Providers::default();
        book.insert(
            "no-terms-yet".to_string(),
            ProviderEntry {
                base_url: "https://not-fetched.example".to_string(),
                terms: None,
                fetched_at: None,
            },
        )
        .unwrap();

        assert!(book.by_provider(&Hash28::from([9u8; 28])).is_empty());
    }
}
