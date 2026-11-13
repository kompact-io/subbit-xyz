//! Named behavior templates ("personas") a `Consumer` is sampled from.
//!
//! A persona only ever configures two independent things: the L2 agent's own
//! pacing/cutoff/catalog tier (`pace`, `amount_cutoff_frac`, `level` -- see
//! `consumer::agent::Agent::spawn`) and L1's fixed Add schedule (`adds`,
//! `add_amount_frac`). Neither reads the other's progress -- see
//! `plan-canary-consumer-spawn.md`'s own "L1 becomes genuinely independent". L1's own
//! exit (`Close`, then `Elapse`) is no longer a persona-configured wall-clock delay --
//! it fires as soon as the schedule above is exhausted, driven by `Round`
//! (`consumer::l1`), so there's nothing left here to configure for it.

use std::ops::RangeInclusive;
use std::time::Duration;

use rand::{Rng, RngExt};

use super::Dialect;

/// A concrete, already-sampled plan for one consumer.
#[derive(Debug, Clone)]
pub struct Behavior {
    pub dialect: Dialect,
    pub deposit: u64,
    /// One entry per `Want::Add` this consumer performs mid-lifecycle, fired
    /// by index against the `Round` `Consumer::run` is handed -- see `consumer::l1`.
    pub add_amounts: Vec<u64>,
    /// Delay between the L2 agent's own spend attempts. `Duration::ZERO` is
    /// the firehose case, not a separate mode.
    pub pace: Duration,
    /// Stops the L2 agent once it's spent this much, lifetime -- `None` for
    /// no cap (spend forever at `pace` until this consumer's `Close` lands
    /// and gate starts returning `Inactive`).
    pub amount_cutoff: Option<u64>,
    /// Which `crate::catalog` route (one of `LEVELS`) this consumer's agent always
    /// pays -- the per-request cost is whatever the shared catalog declares for
    /// it, never a value this crate invents (see `fsm::Event::Accepted`'s own doc).
    pub level: char,
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
    pace: Duration,
    /// Fraction of `deposit` the L2 agent caps itself at, if set.
    amount_cutoff_frac: Option<RangeInclusive<f64>>,
    level: char,
}

impl Persona {
    /// A neutral baseline: fixed 10 ADA deposit, 50/50 dialect, no `Add`s --
    /// closes as soon as `want` is first asked (nothing scheduled to exhaust).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            deposit: 10_000_000..=10_000_000,
            x402_weight: 0.5,
            adds: 0..=0,
            add_amount_frac: 0.1..=0.1,
            pace: Duration::ZERO,
            amount_cutoff_frac: None,
            level: 'i',
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

    pub fn pace(mut self, pace: Duration) -> Self {
        self.pace = pace;
        self
    }

    /// Pins which `crate::catalog` route (one of `LEVELS`) this persona's agent hits.
    pub fn level(mut self, level: char) -> Self {
        self.level = level;
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
    }

    /// A whale: bigger deposit, mostly x402, more/bigger `Add`s, a heavier catalog tier.
    pub fn richy() -> Self {
        Self::new("richy")
            .deposit(90_000_000..=110_000_000)
            .x402_weight(0.8)
            .adds(3..=6)
            .add_amount_frac(0.10..=0.30)
            .level('x')
    }

    /// A deliberately tiny channel, for stress runs constrained by the funding
    /// wallet's balance: enough to exercise every claim path, but locking only
    /// the validator's 2 ADA buffer plus a small amount of channel value.
    pub fn tiny() -> Self {
        Self::new("tiny")
            .deposit(100_000..=100_000)
            .x402_weight(0.5)
            .adds(1..=3)
            .add_amount_frac(0.10..=0.25)
    }

    /// Pure L2 load test: the agent fires back-to-back requests (pace stays
    /// whatever it already was -- `ZERO` by default, the firehose case)
    /// until it's spent its full deposit, then stops on its own. No `Add`s,
    /// so L1 heads for `Close` right away -- there's nothing left to wait for
    /// once the agent caps out.
    pub fn burst(mut self) -> Self {
        self = self.adds(0..=0);
        self.amount_cutoff_frac = Some(1.0..=1.0);
        self
    }

    /// `burst`'s paced opposite: slower, delayed steps instead of flat-out.
    pub fn chunk(self) -> Self {
        self.pace(Duration::from_millis(200))
    }

    /// Spend continuously at a steady (non-zero) cadence, no cap -- the
    /// agent itself just keeps retrying at `pace` through whatever
    /// `InsufficientFunds` rejections it hits while waiting for this
    /// consumer's own scheduled `Add`s to land (see `fsm::transition`: a
    /// rejection doesn't stop the agent, only changes its reported phase).
    /// Samples more/bigger `Add`s than the baseline so there's L1
    /// traffic to race against -- L1 heads for `Close` once that longer
    /// schedule is exhausted, `Round` by `Round`, rather than on any
    /// wall-clock delay of its own.
    pub fn steady(mut self) -> Self {
        self.pace = Duration::from_millis(100);
        self.adds(3..=8)
    }

    /// Looks up a preset by name.
    pub fn by_name(name: &str) -> anyhow::Result<Self> {
        match name {
            "alice" => Ok(Self::alice()),
            "richy" => Ok(Self::richy()),
            "tiny" => Ok(Self::tiny()),
            other => {
                anyhow::bail!("unknown persona {other:?} -- known personas: alice, richy, tiny")
            }
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
        let amount_cutoff = self
            .amount_cutoff_frac
            .as_ref()
            .map(|frac| ((deposit as f64) * rng.random_range(frac.clone())) as u64);
        Behavior {
            dialect,
            deposit,
            add_amounts,
            pace: self.pace,
            amount_cutoff,
            level: self.level,
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
            "steady" => persona.steady(),
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(level), None) if crate::catalog::LEVELS.contains(&level) => {
                        persona.level(level)
                    }
                    _ => anyhow::bail!(
                        "persona spec {spec:?} has an unknown modifier {other:?} -- expected \
                         one of x402, vanilla, burst, chunk, steady, or a single catalog level \
                         ({:?})",
                        crate::catalog::LEVELS
                    ),
                }
            }
        };
    }
    Ok(RosterEntry { persona, count })
}
