//! Resolves a `Constants`' three identity-shaped fields (iou_key,
//! consumer, provider) into labels this console happens to already
//! know, for the "at a glance" widget the Channels tab renders per
//! Constants (see `lineage`'s own doc on why Constants, not keytag, is
//! the primary grouping level). Provider resolution goes one step
//! further and reuses `reconcile::reconcile` to say whether a matched
//! provider's cached Terms is known, and agrees with this Constants.
//!
//! Deliberately produces an owned value with no borrow from
//! `Providers`/`Consumers`/etc (unlike `reconcile::Match<'a>`, which
//! this builds on): a `Brief` lives inside `lineage::ConstantsLines`,
//! which in turn is handed back across the wasm boundary as owned
//! JSON, so nothing here can carry a lifetime into that.

use cardano_sdk::Hash;
use subbit_core::Hash28;
use subbit_l1_dto::Constants;

use crate::channels::reconcile::{self, Divergence};
use crate::channels::status::is_mine;
use crate::{Consumers, IouKeys, Keyring, Providers};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Brief {
    /// `Consumers::effective`'s label for this Constants' `consumer`
    /// vkh, if any entry (independent or `Keyring`-derived) matches.
    pub consumer_label: Option<String>,
    /// `IouKeys::effective`'s label for this Constants' `iou_key`, same
    /// idea.
    pub iou_key_label: Option<String>,
    /// Whether this Constants' `consumer`/`iou_key` matches something
    /// this console's own identity store knows -- `channels::status::
    /// is_mine`'s exact same decision, not re-derived from the two
    /// labels above (which, incidentally, would agree -- but this is
    /// the one place that decision is made, not two).
    pub mine: bool,
    /// Whether this console can actually *sign* as this Constants' own
    /// `consumer` right now -- the one thing that should gate staging
    /// a consumer action (`Line::can`'s own buttons), not `mine` (which
    /// also counts a vkh merely *labelled* via an independent
    /// `Consumers` entry, with no key behind it at all -- e.g. a CIP-30
    /// address saved for display after a session that's long since
    /// disconnected). True iff either:
    /// - a `Keyring` entry flagged `is_consumer` holds this exact vkh
    ///   (`Keyring::signing_key_for_consumer`) -- signable with no
    ///   connection at all, or
    /// - `active_signer` (the frontend's own currently-connected
    ///   backend, CIP-30 or embedded -- a session fact `Ctx` can't
    ///   derive itself, so it's handed in) is this exact vkh.
    ///
    /// This is `ui::wasm::submit`'s own two-source witness lookup, not
    /// a new rule invented here -- see that module's own doc.
    pub can_sign: bool,
    /// Every known provider entry whose cached Terms' `provider` key
    /// matches this Constants' own `provider` -- a `Vec`, not an
    /// `Option`, for the same reason `Providers::by_provider` is (one
    /// provider key can legitimately back more than one labelled
    /// service).
    pub provider_matches: Vec<ProviderMatch>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderMatch {
    pub label: String,
    pub base_url: String,
    /// Whether this entry's Terms has actually been fetched/entered
    /// yet -- a label can exist with nothing cached behind it still.
    pub has_terms: bool,
    /// Empty iff this entry's Terms (when known) fully agrees with the
    /// txo's own Constants.
    pub divergences: Vec<Divergence>,
}

pub fn brief(
    constants: &Constants,
    consumers: &Consumers,
    iou_keys: &IouKeys,
    keyring: &Keyring,
    providers: &Providers,
    active_signer: Option<Hash28>,
) -> Brief {
    let consumer_label = consumers
        .effective(keyring)
        .find(|(_, vkh)| *vkh == *constants.consumer())
        .map(|(label, _)| label.clone());

    let iou_key_label = iou_keys
        .effective(keyring)
        .find(|(_, key)| *key == *constants.iou_key())
        .map(|(label, _)| label.clone());

    let mine = is_mine(constants, consumers, iou_keys, keyring);

    let can_sign = keyring
        .signing_key_for_consumer(Hash::<28>::from(<[u8; 28]>::from(*constants.consumer())))
        .is_some()
        || active_signer == Some(*constants.consumer());

    let provider_matches =
        reconcile::reconcile(constants, providers.by_provider(constants.provider()))
            .into_iter()
            .map(|m| ProviderMatch {
                label: m.label.to_string(),
                base_url: m.entry.base_url.clone(),
                has_terms: m.entry.terms.is_some(),
                divergences: m.divergences,
            })
            .collect();

    Brief {
        consumer_label,
        iou_key_label,
        mine,
        can_sign,
        provider_matches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderEntry;
    use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
    use subbit_l1_dto::Asset;
    use subbit_l2_dto::{Catalog, Cost, Network, ResourceInfo, Terms};

    fn constants(iou_key: VerifyingKey, consumer: Hash28, provider: Hash28) -> Constants {
        Constants::new(
            Tag::from(b"t".to_vec()),
            Asset::Lovelace,
            iou_key,
            consumer,
            provider,
            Duration::from_secs(900),
        )
    }

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
                default: ResourceInfo {
                    cost: Cost::ByRequest(0),
                    description: String::new(),
                    mime_type: String::new(),
                    max_timeout_seconds: 0,
                    service_name: None,
                    tags: None,
                    icon_url: None,
                },
                resources: Vec::new(),
            },
            reference_script: None,
        }
    }

    #[test]
    fn unknown_keys_and_provider_leave_everything_unlabeled() {
        let c = constants(
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
        );
        let b = brief(
            &c,
            &Consumers::default(),
            &IouKeys::default(),
            &Keyring::default(),
            &Providers::default(),
            None,
        );
        assert_eq!(b.consumer_label, None);
        assert_eq!(b.iou_key_label, None);
        assert!(!b.mine);
        assert!(!b.can_sign);
        assert!(b.provider_matches.is_empty());
    }

    #[test]
    fn matched_consumer_and_iou_key_are_labelled() {
        let iou_key = VerifyingKey::from([1u8; 32]);
        let consumer_vkh = Hash28::from([2u8; 28]);
        let c = constants(iou_key, consumer_vkh, Hash28::from([3u8; 28]));

        let keyring = Keyring::default();
        let mut consumers = Consumers::default();
        consumers
            .insert("alice".to_string(), consumer_vkh, &keyring)
            .unwrap();
        let mut iou_keys = IouKeys::default();
        iou_keys
            .insert("alice-iou".to_string(), iou_key, &keyring)
            .unwrap();

        let b = brief(
            &c,
            &consumers,
            &iou_keys,
            &keyring,
            &Providers::default(),
            None,
        );
        assert_eq!(b.consumer_label, Some("alice".to_string()));
        assert_eq!(b.iou_key_label, Some("alice-iou".to_string()));
        assert!(b.mine);
        // An *independent* Consumers entry is just a labelled vkh --
        // no signing key behind it (this test's own `keyring` is
        // empty), and no active signer was handed in either. `mine`
        // says "we recognize this"; `can_sign` says "we could actually
        // sign for it right now" -- they diverge here on purpose.
        assert!(!b.can_sign);
    }

    #[test]
    fn keyring_held_consumer_key_signs_with_no_active_signer_needed() {
        let consumers = Consumers::default();
        let iou_keys = IouKeys::default();
        let mut keyring = Keyring::default();
        // The vkh comes from whatever key `generate` actually minted --
        // flagged `is_consumer` keys don't get to pick their own vkh.
        let consumer_vkh = keyring
            .generate("k".to_string(), true, false, &consumers, &iou_keys)
            .unwrap()
            .consumer_hash();
        let c = constants(
            VerifyingKey::from([1u8; 32]),
            consumer_vkh,
            Hash28::from([3u8; 28]),
        );

        let b = brief(
            &c,
            &consumers,
            &iou_keys,
            &keyring,
            &Providers::default(),
            None,
        );
        assert!(b.can_sign);
    }

    #[test]
    fn active_signer_matching_consumer_signs_even_with_no_label_at_all() {
        let consumer_vkh = Hash28::from([2u8; 28]);
        let c = constants(
            VerifyingKey::from([1u8; 32]),
            consumer_vkh,
            Hash28::from([3u8; 28]),
        );

        let b = brief(
            &c,
            &Consumers::default(),
            &IouKeys::default(),
            &Keyring::default(),
            &Providers::default(),
            Some(consumer_vkh),
        );
        assert_eq!(b.consumer_label, None, "no label at all -- not even `mine`");
        assert!(!b.mine);
        assert!(
            b.can_sign,
            "the active signer's own vkh still makes it signable"
        );
    }

    #[test]
    fn active_signer_for_a_different_vkh_does_not_sign() {
        let c = constants(
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
        );

        let b = brief(
            &c,
            &Consumers::default(),
            &IouKeys::default(),
            &Keyring::default(),
            &Providers::default(),
            Some(Hash28::from([9u8; 28])),
        );
        assert!(!b.can_sign);
    }

    #[test]
    fn matched_provider_reports_label_and_terms_agreement() {
        let provider = Hash28::from([9u8; 28]);
        let c = constants(
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            provider,
        );

        let mut providers = Providers::default();
        providers
            .insert(
                "acme".to_string(),
                ProviderEntry::manual("https://acme.example".to_string(), terms(provider)),
            )
            .unwrap();

        let b = brief(
            &c,
            &Consumers::default(),
            &IouKeys::default(),
            &Keyring::default(),
            &providers,
            None,
        );
        assert_eq!(b.provider_matches.len(), 1);
        assert_eq!(b.provider_matches[0].label, "acme");
        assert!(b.provider_matches[0].has_terms);
        assert!(b.provider_matches[0].divergences.is_empty());
    }
}
