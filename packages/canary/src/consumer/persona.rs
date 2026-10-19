//! Named behavior templates ("personas") a `Consumer` can be built from,
//! and the deterministic random sampling that turns one into a concrete
//! `Behavior` -- so a scenario can say "5 richy, 2 alice, 3 richy but
//! x402-only" instead of hand-picking a deposit/dialect/add-count for
//! every single consumer.

use std::ops::RangeInclusive;
use std::time::Duration;

use rand::{Rng, RngExt};

use super::Dialect;

/// A concrete, already-sampled plan for one consumer: what `ConsumerBuilder`
/// turns into actual `Consumer` fields, and what the persona-driven
/// scenario reads back to know what actions to run.
#[derive(Debug, Clone)]
pub struct Behavior {
    pub dialect: Dialect,
    pub deposit: u64,
    /// One entry per `Want::Add` this consumer performs mid-lifecycle,
    /// in order.
    pub add_amounts: Vec<u64>,
    /// One entry per spend round -- `spend_counts.len() ==
    /// add_amounts.len() + 1`, since the last round has no `Add`
    /// following it, just the eventual `Close`.
    pub spend_counts: Vec<usize>,
    /// Delay between each spend request within a round -- `ZERO` (the
    /// default) fires every request back to back, as fast as the local
    /// provider accepts them; a nonzero pace spreads the same loop out
    /// in real time instead (see `Persona::chunk`).
    pub pace: Duration,
    /// Set by `Persona::burst`: overrides `spend_counts`/`pace` entirely
    /// for that one (and only) round -- see `BurstStop`.
    pub burst: Option<BurstStop>,
}

/// A sampled stop condition for `Persona::burst`'s spend loop: run until
/// `deadline` or `commit_max`, whichever comes first. See
/// `Consumer::firehose_l2`'s own burst branch.
#[derive(Debug, Clone, Copy)]
pub struct BurstStop {
    pub deadline: Duration,
    pub commit_max: u64,
}

/// A named behavior template: ranges and weights `generate` samples from,
/// not concrete values -- the same `Persona` produces a different
/// `Behavior` for every consumer index it's applied to.
#[derive(Debug, Clone)]
pub struct Persona {
    pub name: String,
    deposit: RangeInclusive<u64>,
    /// Probability of `Dialect::X402` over `Dialect::Vanilla` when
    /// sampling -- `1.0`/`0.0` pins a single dialect (see
    /// `Persona::dialect_only`).
    x402_weight: f64,
    adds: RangeInclusive<u32>,
    /// Each `Add`'s amount, as a fraction of `deposit`.
    add_amount_frac: RangeInclusive<f64>,
    spends_per_round: RangeInclusive<u32>,
    /// See `Behavior::pace`.
    pace: Duration,
    /// Set by `burst`: deadline plus the fraction of the sampled
    /// deposit to spend up to. `None` for every other persona/modifier.
    burst: Option<(Duration, RangeInclusive<f64>)>,
}

impl Persona {
    /// A neutral baseline: fixed 10 ADA deposit, 50/50 dialect, no
    /// `Add`s, one spend before close. Presets below (`alice`, `richy`)
    /// build on this rather than repeating every field.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            deposit: 10_000_000..=10_000_000,
            x402_weight: 0.5,
            adds: 0..=0,
            add_amount_frac: 0.1..=0.1,
            spends_per_round: 1..=1,
            pace: Duration::ZERO,
            burst: None,
        }
    }

    pub fn deposit(mut self, range: RangeInclusive<u64>) -> Self {
        self.deposit = range;
        self
    }

    pub fn x402_weight(mut self, weight: f64) -> Self {
        self.x402_weight = weight;
        self
    }

    pub fn adds(mut self, range: RangeInclusive<u32>) -> Self {
        self.adds = range;
        self
    }

    pub fn add_amount_frac(mut self, range: RangeInclusive<f64>) -> Self {
        self.add_amount_frac = range;
        self
    }

    pub fn spends_per_round(mut self, range: RangeInclusive<u32>) -> Self {
        self.spends_per_round = range;
        self
    }

    pub fn pace(mut self, pace: Duration) -> Self {
        self.pace = pace;
        self
    }

    /// Pins this persona to a single dialect -- e.g. "richy, but
    /// x402-only" -- by collapsing `x402_weight` to `1.0`/`0.0` rather
    /// than sampling it.
    pub fn dialect_only(self, dialect: Dialect) -> Self {
        match dialect {
            Dialect::X402 => self.x402_weight(1.0),
            Dialect::Vanilla => self.x402_weight(0.0),
        }
    }

    /// A modest, middle-of-the-road user: ~20 ADA deposit, 50/50
    /// dialect, 1-3 top-ups, light spend between each.
    pub fn alice() -> Self {
        Self::new("alice")
            .deposit(18_000_000..=22_000_000)
            .x402_weight(0.5)
            .adds(1..=3)
            .add_amount_frac(0.10..=0.25)
            .spends_per_round(1..=5)
    }

    /// A whale: ~100 ADA deposit, mostly x402, more top-ups, and much
    /// heavier spend per round.
    pub fn richy() -> Self {
        Self::new("richy")
            .deposit(90_000_000..=110_000_000)
            .x402_weight(0.8)
            .adds(3..=6)
            .add_amount_frac(0.10..=0.30)
            .spends_per_round(10..=30)
    }

    /// Pure L2 load test: back-to-back requests, no `Add`s, until a
    /// deadline or a sampled fraction of deposit is committed. See
    /// `BurstStop`, `Consumer::firehose_l2`, `parse_roster`'s `:burst`.
    pub fn burst(self) -> Self {
        let mut this = self.adds(0..=0);
        this.burst = Some((Duration::from_secs(10), 0.5..=0.9));
        this
    }

    /// `burst`'s paced opposite: fewer, larger steps with a real delay
    /// between requests. ~14x slower than `burst`'s rate -- match
    /// `--flat-cost` by that factor to compare fund-flow rates fairly.
    /// See `parse_roster`'s `:chunk` modifier.
    pub fn chunk(self) -> Self {
        self.spends_per_round(5..=10)
            .pace(Duration::from_millis(200))
    }

    /// Looks up a preset by name -- the CLI roster syntax (`persona::parse_roster`)
    /// goes through this rather than matching persona names in two places.
    pub fn by_name(name: &str) -> anyhow::Result<Self> {
        match name {
            "alice" => Ok(Self::alice()),
            "richy" => Ok(Self::richy()),
            other => anyhow::bail!("unknown persona {other:?} -- known personas: alice, richy"),
        }
    }

    /// Samples one concrete `Behavior` from this template.
    pub fn generate(&self, rng: &mut impl Rng) -> Behavior {
        let deposit = rng.random_range(self.deposit.clone());
        let dialect = if rng.random_bool(self.x402_weight) {
            Dialect::X402
        } else {
            Dialect::Vanilla
        };
        let add_count = rng.random_range(self.adds.clone());
        let add_amounts: Vec<u64> = (0..add_count)
            .map(|_| {
                let frac = rng.random_range(self.add_amount_frac.clone());
                ((deposit as f64) * frac) as u64
            })
            .collect();
        let rounds = add_amounts.len() + 1;
        let spend_counts: Vec<usize> = (0..rounds)
            .map(|_| rng.random_range(self.spends_per_round.clone()) as usize)
            .collect();
        let burst = self
            .burst
            .as_ref()
            .map(|(deadline, commit_max_frac)| BurstStop {
                deadline: *deadline,
                commit_max: ((deposit as f64) * rng.random_range(commit_max_frac.clone())) as u64,
            });
        Behavior {
            dialect,
            deposit,
            add_amounts,
            spend_counts,
            pace: self.pace,
            burst,
        }
    }
}

/// One roster line: `count` consumers built from `persona`.
pub struct RosterEntry {
    pub persona: Persona,
    pub count: usize,
}

/// Parses `--persona` specs of the form `<name>[:<modifier>]*=<count>`,
/// e.g. `alice=5`, `richy:x402=3` (richy, x402-only, 3 of them),
/// `alice:burst=2` (alice, but hundreds of spends in one round instead
/// of her usual modest few -- see `Persona::burst`), `richy:x402:burst=1`
/// (both at once -- modifiers apply left to right, in any order).
/// Recognized modifiers: `x402`, `vanilla` (dialect), `burst` (fast,
/// high-volume spend), `chunk` (paced, moderate-volume spend -- see
/// `Persona::burst`/`::chunk` for how to compare the two fairly). Falls
/// back to a sensible default roster (`alice=3 richy=1`)
/// when `specs` is empty, so a scenario always has *something* to drive
/// without the caller enumerating every option.
pub fn parse_roster(specs: &[String]) -> anyhow::Result<Vec<RosterEntry>> {
    if specs.is_empty() {
        return Ok(vec![
            RosterEntry {
                persona: Persona::alice(),
                count: 3,
            },
            RosterEntry {
                persona: Persona::richy(),
                count: 1,
            },
        ]);
    }
    specs.iter().map(|spec| parse_one(spec)).collect()
}

fn parse_one(spec: &str) -> anyhow::Result<RosterEntry> {
    let (name_part, count_str) = spec
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("persona spec {spec:?} is missing '=<count>'"))?;
    let count: usize = count_str
        .parse()
        .map_err(|e| anyhow::anyhow!("persona spec {spec:?} has a bad count: {e}"))?;
    let mut parts = name_part.split(':');
    let name = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("persona spec {spec:?} is missing a persona name"))?;
    let mut persona = Persona::by_name(name)?;
    for modifier in parts {
        persona = match modifier {
            "x402" => persona.dialect_only(Dialect::X402),
            "vanilla" => persona.dialect_only(Dialect::Vanilla),
            "burst" => persona.burst(),
            "chunk" => persona.chunk(),
            other => anyhow::bail!(
                "persona spec {spec:?} has an unknown modifier {other:?} -- expected one of \
                 x402, vanilla, burst, chunk"
            ),
        };
    }
    Ok(RosterEntry { persona, count })
}
