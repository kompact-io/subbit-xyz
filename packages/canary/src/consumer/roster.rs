//! Turns a `--persona` roster into planned consumers (identity, tag, sampled behavior).

use rand::SeedableRng;
use rand::rngs::StdRng;
use subbit_core::Tag;

use crate::identity::{self, Identity};

use super::persona::{self, Behavior};

/// Consumer index `i`'s identity material, used by both a roster seat and a tidy placeholder.
pub(crate) fn identity_for(l2_seed: &[u8; 32], i: usize) -> (Identity, [u8; 32], Tag) {
    let seed = identity::consumer_seed(l2_seed, i);
    let tag = identity::tag_for(l2_seed, i);
    (Identity::from_seed(seed), seed, tag)
}

/// Seeds the one `StdRng` every persona samples from, so `L2_SEED_HEX` reproduces behaviors too.
fn rng_from_seed(l2_seed: &[u8; 32]) -> StdRng {
    let derived = identity::derive_seed(l2_seed, "persona-rng");
    StdRng::seed_from_u64(u64::from_le_bytes(derived[..8].try_into().unwrap()))
}

/// One roster seat, fully planned: identity, tag, and a sampled `Behavior`.
pub(crate) struct PlannedConsumer {
    pub identity: Identity,
    pub seed: [u8; 32],
    pub tag: Tag,
    pub behavior: Behavior,
}

/// Parses `roster_specs` and samples each seat's own concrete `Behavior`.
pub(crate) fn plan_roster(
    l2_seed: &[u8; 32],
    roster_specs: &[String],
    burst_target_lovelace: Option<u64>,
) -> anyhow::Result<Vec<PlannedConsumer>> {
    let roster = persona::parse_roster(roster_specs)?;
    anyhow::ensure!(
        roster.iter().map(|e| e.count).sum::<usize>() > 0,
        "persona roster has zero total consumers"
    );

    let personas: Vec<&persona::Persona> = roster
        .iter()
        .flat_map(|entry| std::iter::repeat_n(&entry.persona, entry.count))
        .collect();

    let mut rng = rng_from_seed(l2_seed);
    personas
        .into_iter()
        .enumerate()
        .map(|(i, persona)| {
            let (identity, seed, tag) = identity_for(l2_seed, i);
            let mut behavior = persona.generate(&mut rng);
            if let (Some(target), Some(cutoff)) =
                (burst_target_lovelace, behavior.amount_cutoff.as_mut())
            {
                *cutoff = target;
            }
            println!(
                "canary: [{i}] {} -- dialect={:?} level={:?} deposit={} adds={:?} pace={:?} \
                 amount_cutoff={:?}",
                persona.name,
                behavior.dialect,
                behavior.level,
                behavior.deposit,
                behavior.add_amounts,
                behavior.pace,
                behavior.amount_cutoff,
            );
            Ok(PlannedConsumer {
                identity,
                seed,
                tag,
                behavior,
            })
        })
        .collect()
}
