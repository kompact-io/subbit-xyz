use std::collections::BTreeMap;
use std::path::Path;

use cardano_sdk::{Input, Output};
use serde::{Deserialize, Serialize};
use subbit_core::{Constants, Currency, Duration, Hash28, Stage};
use subbit_tx::{Channel, Variables};
use tracing::warn;
use url::Url;

use crate::wire::{Backing, Keytag, Row};

/// Config for `naive` mode: connect a session, poll on an interval, and
/// report every channel matching (provider, currency), with a close_period
/// at least `close_period` and a tag no longer than `max_tag_len`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub session: subbit_session::session::Config,
    pub endpoint: Url,
    pub provider: Hash28,
    pub currency: Currency,
    /// Minimum close_period a channel must have to match.
    pub close_period: Duration,
    /// Maximum tag length (bytes) a channel's tag may have to match.
    pub max_tag_len: usize,
    pub poll_interval_secs: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] toml::de::Error),
    #[error(transparent)]
    Serialize(#[from] toml::ser::Error),
}

impl Default for Config {
    fn default() -> Self {
        Self {
            session: subbit_session::session::Config::default(),
            endpoint: "http://127.0.0.1:7822/v1/a/backings"
                .parse()
                .expect("valid url"),
            provider: Hash28::from([0; 28]),
            currency: Currency::Ada,
            close_period: Duration::from_millis(3_600_000),
            max_tag_len: 32,
            poll_interval_secs: 30,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Error> {
        Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn write_default(path: &Path) -> Result<(), Error> {
        std::fs::write(path, toml::to_string_pretty(&Self::default())?)?;
        Ok(())
    }
}

fn keytag_of(constants: &Constants) -> Keytag {
    let mut bytes = constants.iou_key().to_bytes().to_vec();
    bytes.extend_from_slice(constants.tag().as_ref());
    Keytag(bytes)
}

fn matches(constants: &Constants, cfg: &Config) -> bool {
    constants.provider() == &cfg.provider
        && constants.currency() == &cfg.currency
        && constants.verify(cfg.max_tag_len, cfg.close_period.as_millis() as u64)
}

/// Among a keytag's Opened variables, amount != 0 wins, tie-broken by
/// lowest subbed. `false` sorts before `true`, so `(amount == 0, subbed)`
/// puts nonzero-amount channels first, ascending on subbed within that.
/// Anything not Opened (Closed, Settled) isn't a candidate.
fn opened_sort_key(v: &Variables) -> Option<(bool, u64)> {
    match v.stage() {
        Stage::Opened { subbed } => Some((v.amount() == 0, *subbed)),
        _ => None,
    }
}

/// Groups channel outputs matching `cfg` by keytag. Outputs that aren't
/// valid channels are logged and skipped.
fn group_matching(
    channels: &BTreeMap<Input, Output>,
    cfg: &Config,
) -> BTreeMap<Keytag, Vec<Variables>> {
    let mut groups: BTreeMap<Keytag, Vec<Variables>> = BTreeMap::new();
    for output in channels.values() {
        let channel = match Channel::try_from(output) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "skipping output: not a valid channel");
                continue;
            }
        };
        if matches(channel.constants(), cfg) {
            groups
                .entry(keytag_of(channel.constants()))
                .or_default()
                .push(channel.variables().to_owned());
        }
    }
    groups
}

/// Picks the best Opened candidate for a keytag, if any. `None` means the
/// keytag has no Opened channel (Closed or Settled), i.e. a closed Row.
fn select_backing(vars: &[Variables]) -> Option<Backing> {
    vars.iter()
        .filter_map(|v| opened_sort_key(v).map(|key| (key, v)))
        .min_by_key(|(key, _)| *key)
        .map(|(_, v)| match v.stage() {
            Stage::Opened { subbed } => Backing {
                amount: v.amount(),
                subbed: *subbed,
            },
            _ => unreachable!("opened_sort_key only returns Some for Opened"),
        })
}

/// Builds one `Row` per distinct keytag among channels matching `cfg`.
/// Multiple UTXOs can share a keytag (no on-chain uniqueness enforced,
/// though rare in practice); see `select_backing` for how ties resolve.
pub fn rows_from_channels(channels: &BTreeMap<Input, Output>, cfg: &Config) -> Vec<Row> {
    group_matching(channels, cfg)
        .into_iter()
        .map(|(keytag, vars)| {
            let backing = select_backing(&vars);
            Row {
                keytag,
                amount: backing.as_ref().map(|b| b.amount),
                subbed: backing.as_ref().map(|b| b.subbed),
            }
        })
        .collect()
}
