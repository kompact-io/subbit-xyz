use std::cmp;

use minicbor::{Decode, Encode};
use subbit_core::{Duration, Instant};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "web"))]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "web")]
use web_time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Bounds {
    #[n(0)]
    pub lower: Option<Instant>,
    #[n(1)]
    pub upper: Option<Instant>,
}

impl Bounds {
    pub fn lower(lower: Instant) -> Self {
        Self {
            lower: Some(lower),
            upper: None,
        }
    }

    pub fn upper(upper: Instant) -> Self {
        Self {
            lower: None,
            upper: Some(upper),
        }
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let lower = match (self.lower, other.lower) {
            (Some(a), Some(b)) => Some(cmp::max(a, b)),
            (a, b) => a.or(b),
        };
        let upper = match (self.upper, other.upper) {
            (Some(a), Some(b)) => Some(cmp::min(a, b)),
            (a, b) => a.or(b),
        };

        Self { lower, upper }
    }

    /// `false` once `lower`/`upper` can never both hold at once -- the
    /// one way combining two individually-fine bounds (via `intersect`)
    /// can go wrong: each `Will`'s own bounds are always individually
    /// feasible (`lower`/`upper` only ever come from one side), so this
    /// only turns up once two *different* `Will`s end up in the same
    /// batch with disjoint windows.
    pub fn is_feasible(&self) -> bool {
        !matches!((self.lower, self.upper), (Some(l), Some(u)) if l > u)
    }

    /// `true` once this bound's own `upper` has already passed `now` --
    /// unlike plain infeasibility (`is_feasible`), this can never
    /// resolve itself later: `upper` doesn't move, so a `Will` this is
    /// true for is permanently undoable, not just unlucky this batch.
    pub fn has_expired(&self, now: Instant) -> bool {
        self.upper.is_some_and(|upper| upper < now)
    }

    pub fn twenty_mins() -> Self {
        // TODO :: Either use std time, or upstream methods
        let lower = Instant::from_secs(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                // Hack to handle blockfrost slots not aligning with current time.
                .saturating_sub(60),
        );
        let upper = lower + Duration::from_secs(19 * 60);
        Bounds {
            lower: Some(lower),
            upper: Some(upper),
        }
    }
}
