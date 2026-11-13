//! A txo's status as seen by this particular consumer: whether it's
//! theirs (`mine`), and which actions are currently available (`can`,
//! filtered to the consumer-relevant subset of `Channel::can` --
//! `Sub`/`Settle` are provider-only, out of Console's own scope).
//! Decided once, here, so every frontend (the browser app, and
//! whatever consumes this next) reads the answer instead of
//! re-deriving it.

use cardano_sdk::Input;
use subbit_l1_dto::Constants;
use subbit_tx::{Channel, Variables, step::Can};

use crate::{Consumers, IouKeys, Keyring};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Status {
    pub input: Input,
    pub constants: Constants,
    pub variables: Variables,
    pub mine: bool,
    pub can: Vec<Can>,
}

pub fn statuses<'a>(
    txos: impl IntoIterator<Item = (&'a Input, &'a Channel)>,
    consumers: &Consumers,
    iou_keys: &IouKeys,
    keyring: &Keyring,
) -> Vec<Status> {
    txos.into_iter()
        .map(|(input, txo)| Status {
            input: input.clone(),
            constants: txo.constants().clone(),
            variables: txo.variables().clone(),
            mine: is_mine(txo.constants(), consumers, iou_keys, keyring),
            can: consumer_can(txo),
        })
        .collect()
}

/// The subset of `Channel::can()` that's actually a consumer's own to
/// act on -- `Sub`/`Settle` are provider-only, out of Console's own
/// scope (see this module's own doc). `pub(crate)` so `channels::
/// lineage` can attach the exact same decision to a line's own tip,
/// rather than re-deriving it.
pub(crate) fn consumer_can(channel: &Channel) -> Vec<Can> {
    channel
        .can()
        .into_iter()
        .filter(|c| matches!(c, Can::Add | Can::Close | Can::Elapse { .. } | Can::End))
        .collect()
}

pub(crate) fn is_mine(
    constants: &Constants,
    consumers: &Consumers,
    iou_keys: &IouKeys,
    keyring: &Keyring,
) -> bool {
    consumers
        .effective(keyring)
        .any(|(_, vkh)| vkh == *constants.consumer())
        || iou_keys
            .effective(keyring)
            .any(|(_, key)| key == *constants.iou_key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_sdk::Hash;
    use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
    use subbit_l1_dto::{Asset, Stage};

    fn txo(consumer: Hash28, iou_key: VerifyingKey) -> Channel {
        Channel::new(
            Constants::new(
                Tag::from(b"t".to_vec()),
                Asset::Lovelace,
                iou_key,
                consumer,
                Hash28::from([9u8; 28]),
                Duration::from_secs(900),
            ),
            Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
        )
    }

    #[test]
    fn opened_txo_offers_add_and_close_but_not_sub_or_elapse() {
        let keyring = Keyring::default();
        let consumers = Consumers::default();
        let iou_keys = IouKeys::default();
        let txo = txo(Hash28::from([1u8; 28]), VerifyingKey::from([2u8; 32]));
        let input = Input::new(Hash::<32>::from([3u8; 32]), 0);

        let statuses = statuses([(&input, &txo)], &consumers, &iou_keys, &keyring);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].can, vec![Can::Add, Can::Close]);
    }

    #[test]
    fn matching_consumer_vkh_is_mine() {
        let keyring = Keyring::default();
        let mut consumers = Consumers::default();
        let vkh = Hash28::from([1u8; 28]);
        consumers.insert("me".to_string(), vkh, &keyring).unwrap();
        let iou_keys = IouKeys::default();
        let txo = txo(vkh, VerifyingKey::from([2u8; 32]));
        let input = Input::new(Hash::<32>::from([3u8; 32]), 0);

        let statuses = statuses([(&input, &txo)], &consumers, &iou_keys, &keyring);

        assert!(statuses[0].mine);
    }

    #[test]
    fn unrelated_txo_is_not_mine() {
        let keyring = Keyring::default();
        let consumers = Consumers::default();
        let iou_keys = IouKeys::default();
        let txo = txo(Hash28::from([1u8; 28]), VerifyingKey::from([2u8; 32]));
        let input = Input::new(Hash::<32>::from([3u8; 32]), 0);

        let statuses = statuses([(&input, &txo)], &consumers, &iou_keys, &keyring);

        assert!(!statuses[0].mine);
    }
}
