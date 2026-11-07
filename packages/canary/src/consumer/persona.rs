//! Named behavior templates ("personas") a `Consumer` is sampled from.

use std::ops::RangeInclusive;
use std::time::Duration;

use rand::{Rng, RngExt};

use super::Dialect;

/// A concrete, already-sampled plan for one consumer.
#[derive(Debug, Clone)]
pub struct Behavior {
    pub dialect: Dialect,
    pub deposit: u64,
    /// One entry per `Want::Add` this consumer performs mid-lifecycle.
    pub add_amounts: Vec<u64>,
    /// One entry per spend round.
    pub spend_counts: Vec<usize>,
    /// Delay between each spend request within a round.
    pub pace: Duration,
    /// Overrides `spend_counts`/`pace` for one round -- see `Persona::burst`.
    pub burst: Option<BurstStop>,
}

/// A sampled stop condition for `Persona::burst`'s spend loop.
#[derive(Debug, Clone, Copy)]
pub struct BurstStop {
    pub deadline: Duration,
    pub commit_max: u64,
    pub close_after: Duration,
}

/// A named behavior template: ranges and weights `generate` samples from.
#[derive(Debug, Clone)]
pub struct Persona {
    pub name: String,
    deposit: RangeInclusive<u64>,
    /// Probability of `Dialect::X402` over `Dialect::Vanilla`.
    x402_weight: f64,
    adds: RangeInclusive<u32>,
    /// Each `Add`'s amount, as a fraction of `deposit`.
    add_amount_frac: RangeInclusive<f64>,
    spends_per_round: RangeInclusive<u32>,
    pace: Duration,
    /// Set by `burst`: deadline, commit-max fraction, close_after.
    burst: Option<(Duration, RangeInclusive<f64>, Duration)>,
}

impl Persona {
    /// A neutral baseline: fixed 10 ADA deposit, 50/50 dialect, no `Add`s.
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

    /// Pins this persona to a single dialect.
    pub fn dialect_only(self, dialect: Dialect) -> Self {
        match dialect {
            Dialect::X402 => self.x402_weight(1.0),
            Dialect::Vanilla => self.x402_weight(0.0),
        }
    }

    /// A modest, middle-of-the-road user.
    pub fn alice() -> Self {
        Self::new("alice")
            .deposit(18_000_000..=22_000_000)
            .x402_weight(0.5)
            .adds(1..=3)
            .add_amount_frac(0.10..=0.25)
            .spends_per_round(1..=5)
    }

    /// A whale: bigger deposit, mostly x402, heavier spend.
    pub fn richy() -> Self {
        Self::new("richy")
            .deposit(90_000_000..=110_000_000)
            .x402_weight(0.8)
            .adds(3..=6)
            .add_amount_frac(0.10..=0.30)
            .spends_per_round(10..=30)
    }

    /// Pure L2 load test: back-to-back requests until deposit committed, no
    /// deadline (`commit_max` is the only stop condition). Uses a 10-year
    /// "deadline" rather than `Duration::MAX` -- `Instant + Duration::MAX`
    /// panics on overflow in `spend_burst`.
    // commit_max is the full deposit, not a fraction -- firehose_l2 reinvokes in
    // deadline-sized slices, so a fraction would thin concurrency over the run.
    pub fn burst(self) -> Self {
        let mut this = self.adds(0..=0);
        this.burst = Some((
            Duration::from_secs(10 * 365 * 24 * 3600),
            1.0..=1.0,
            Duration::from_secs(45),
        ));
        this
    }

    /// `burst`'s paced opposite: fewer, larger, delayed steps.
    pub fn chunk(self) -> Self {
        self.spends_per_round(5..=10)
            .pace(Duration::from_millis(200))
    }

    /// Looks up a preset by name.
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
            .map(|(deadline, commit_max_frac, close_after)| BurstStop {
                deadline: *deadline,
                commit_max: ((deposit as f64) * rng.random_range(commit_max_frac.clone())) as u64,
                close_after: *close_after,
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

/// Parses `--persona` specs of the form `<name>[:<modifier>]*=<count>`.
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
