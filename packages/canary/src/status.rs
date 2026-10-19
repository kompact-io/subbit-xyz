//! Read-only census of the validator address: total UTxOs, how many
//! decode as channels this run recognizes, and how those recognized
//! channels split across `Stage`.

use std::collections::BTreeMap;

use anyhow::Context;
use cardano_connector::CardanoConnector;
use cardano_sdk::Input;
use subbit_core::Hash28;
use subbit_l1_dto::Stage;

use crate::identity;
use crate::l1::L1;

pub async fn run<C: CardanoConnector>(
    l2_seed: [u8; 32],
    scan_consumers: usize,
    mut l1: L1<C>,
) -> anyhow::Result<()> {
    let seeds = known_seeds(&l2_seed, scan_consumers);
    let identity_to_seed = identity_to_seed_map(&seeds);

    let recognition = recognize_channels(&mut l1, &identity_to_seed).await?;

    let mut opened = 0;
    let mut closed = 0;
    let mut settled = 0;
    for (_, channel, _) in &recognition.recognized {
        match channel.variables().stage() {
            Stage::Opened { .. } => opened += 1,
            Stage::Closed { .. } => closed += 1,
            Stage::Settled => settled += 1,
        }
    }

    println!(
        "canary status: {} utxo(s) at the validator address",
        recognition.total
    );
    println!(
        "canary status: {} belong to this run's keys ({} unrecognized -- not this run's, or \
         --scan-consumers {scan_consumers} is too small to cover them)",
        recognition.recognized.len(),
        recognition.unrecognized_count()
    );
    println!("canary status:   Opened:  {opened}");
    println!("canary status:   Closed:  {closed}");
    println!("canary status:   Settled: {settled}");

    Ok(())
}

/// Seeds used before `L2_SEED_HEX`/`Keyring` existed (see `results.md`'s
/// "Run 1"/"Run 2" notes) -- kept so status can still find and report
/// channels opened under them. `seed[0] = 100 + i` only ever had a
/// `u8`'s worth of room (`100..=255`), so this covers at most 156
/// indices regardless of `scan_consumers` -- that's the legacy scheme's
/// real domain, not something widening `scan_consumers` can extend.
fn legacy_seeds(scan_consumers: usize) -> Vec<[u8; 32]> {
    let mut seeds = vec![[7u8; 32]];
    for i in 0..scan_consumers.min(156) {
        let mut seed = [0u8; 32];
        seed[0] = 100 + i as u8;
        seeds.push(seed);
    }
    seeds
}

/// Every consumer/provider identity this run is willing to recognize --
/// `scan_consumers` is an upper bound on consumer *index*, independent
/// of how many consumers any particular run actually drove, so it
/// should generally be set generously: a channel at an index past this
/// bound is silently invisible (see `recognize_channels`'s own
/// unrecognized-count print for how that shows up instead of silent
/// success).
fn known_seeds(l2_seed: &[u8; 32], scan_consumers: usize) -> Vec<[u8; 32]> {
    let mut seeds = vec![identity::provider_seed(l2_seed)];
    for i in 0..scan_consumers {
        seeds.push(identity::consumer_seed(l2_seed, i));
    }
    seeds.extend(legacy_seeds(scan_consumers));
    seeds
}

fn identity_to_seed_map(seeds: &[[u8; 32]]) -> BTreeMap<Hash28, [u8; 32]> {
    seeds
        .iter()
        .map(|s| (identity::hash28_from_seed(s), *s))
        .collect()
}

/// Every UTxO currently at the validator address, split into the ones
/// that decode as a channel owned by one of `identity_to_seed`'s keys
/// (`recognized`) and everything else (`unrecognized_count`) -- a
/// non-zero `unrecognized_count` is not necessarily a problem (it may
/// genuinely be someone else's channel), but it's exactly what a too-
/// small `scan_consumers` looks like too, so this surfaces it rather
/// than silently reading as "nothing here".
struct Recognition {
    total: usize,
    recognized: Vec<(Input, subbit_tx::Channel, [u8; 32])>,
}

impl Recognition {
    fn unrecognized_count(&self) -> usize {
        self.total - self.recognized.len()
    }
}

async fn recognize_channels<C: CardanoConnector>(
    l1: &mut L1<C>,
    identity_to_seed: &BTreeMap<Hash28, [u8; 32]>,
) -> anyhow::Result<Recognition> {
    l1.refresh().await.context("refreshing tracked channels")?;

    let utxos = l1.channels();
    let total = utxos.len();
    let recognized = utxos
        .into_iter()
        .filter_map(|(input, output)| {
            let channel = subbit_tx::Channel::try_from(&output).ok()?;
            let seed = *identity_to_seed.get(channel.constants().consumer())?;
            Some((input, channel, seed))
        })
        .collect();

    Ok(Recognition { total, recognized })
}
