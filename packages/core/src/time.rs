use core::{
    fmt,
    ops::{Add, Deref, DerefMut, Mul, Sub},
    str::FromStr,
    time,
};

use crate::ParseError;

/// A relative amount of time: a thin newtype over `std::time::Duration`,
/// purely to carry this crate's own serde/cbor encoding (milliseconds,
/// as a bare `u64`). Opposite number to [`Instant`], an *absolute*
/// point in time.
#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Duration(time::Duration);

/// A point in time: milliseconds since the Posix epoch -- the same
/// quantity `std::time::SystemTime::duration_since(UNIX_EPOCH)` would
/// hand back, stored directly rather than going through `SystemTime`
/// itself (`SystemTime`/`std::time::Instant` are both platform clock
/// types with no raw constructor from a stored/decoded value, and
/// `SystemTime` in particular is a known wasm trouble spot -- this
/// crate never touches either, every `now()` lives in a caller with
/// its own wasm-safe clock access, e.g. `web_time`, which then hands
/// this type a plain millis value).
///
/// Opposite number to [`Duration`], a *relative* span:
/// `Instant + Duration = Instant`, `Instant - Duration = Instant`,
/// `Instant - Instant = Duration`. Adding or scaling two `Instant`s
/// together is never meaningful, so neither is implemented -- the same
/// shape `std::time::SystemTime`'s own arithmetic takes.
#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Instant(time::Duration);

macro_rules! millis_wire {
    ($ty:ident) => {
        impl $ty {
            pub fn from_secs(secs: u64) -> Self {
                Self(time::Duration::from_secs(secs))
            }

            pub fn from_millis(millis: u64) -> Self {
                Self(time::Duration::from_millis(millis))
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}ms", self.0.as_millis())
            }
        }

        /// Deref onto `std::time::Duration` so read-only methods
        /// (`as_millis`, `as_secs`, ...) need no explicit conversion.
        impl Deref for $ty {
            type Target = time::Duration;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        /// The idiomatic way in: wrapping a `std::time::Duration` this
        /// crate didn't itself produce (e.g. one measured via
        /// `web_time`/`std::time::SystemTime::duration_since`).
        impl From<time::Duration> for $ty {
            fn from(value: time::Duration) -> Self {
                Self(value)
            }
        }

        /// The idiomatic way out: handing this back to an API that
        /// wants a plain `std::time::Duration`.
        impl From<$ty> for time::Duration {
            fn from(value: $ty) -> Self {
                value.0
            }
        }

        /// Converting to `u64`, assuming milliseconds.
        impl From<&$ty> for u64 {
            fn from(value: &$ty) -> Self {
                value.0.as_millis() as u64
            }
        }

        /// Converting to `u64`, assuming milliseconds.
        impl From<$ty> for u64 {
            fn from(value: $ty) -> Self {
                value.0.as_millis() as u64
            }
        }

        impl<C> minicbor::Encode<C> for $ty {
            fn encode<W: minicbor::encode::Write>(
                &self,
                e: &mut minicbor::Encoder<W>,
                ctx: &mut C,
            ) -> Result<(), minicbor::encode::Error<W::Error>> {
                e.encode_with(self.0.as_millis() as u64, ctx)?;
                Ok(())
            }
        }

        impl<'b, C> minicbor::Decode<'b, C> for $ty {
            fn decode(
                d: &mut minicbor::Decoder<'b>,
                ctx: &mut C,
            ) -> Result<Self, minicbor::decode::Error> {
                let millis: u64 = d.decode_with(ctx)?;
                Ok(Self(time::Duration::from_millis(millis)))
            }
        }

        #[cfg(feature = "test-utils")]
        impl proptest::arbitrary::Arbitrary for $ty {
            type Parameters = ();
            type Strategy = proptest::strategy::BoxedStrategy<Self>;
            fn arbitrary_with(_: ()) -> Self::Strategy {
                use proptest::prelude::*;
                any::<u64>().prop_map($ty::from_millis).boxed()
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $ty {
            fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                s.serialize_u64(u64::from(self))
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D>(d: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let millis = u64::deserialize(d)?;
                Ok($ty::from_millis(millis))
            }
        }
    };
}

millis_wire!(Duration);
millis_wire!(Instant);

/// Parsing a relative span from a string slice with a unit postfix
/// (e.g. "210s"). No equivalent exists for `Instant`: an absolute
/// instant has no natural short-hand text form, every caller that
/// needs one resolves it from a parsed `Duration` plus `now()` instead
/// (see e.g. `subbit_cli::ui::prompt::prompt_relative_duration`).
impl FromStr for Duration {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let value: u64 = s
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()?;

        let unit = s
            .chars()
            .skip_while(|c| c.is_ascii_digit())
            .collect::<String>();

        let duration = match unit.as_str() {
            "ms" => time::Duration::from_millis(value),
            "s" => time::Duration::from_secs(value),
            "min" => time::Duration::from_secs(value * 60),
            "h" => time::Duration::from_secs(value * 3600),
            _ => {
                return Err(ParseError::Constraint(
                    "unknown time unit; try one of: 'ms', 's', 'min' or 'h'",
                ));
            }
        };
        Ok(Duration(duration))
    }
}

/// `rel + rel = rel`, via `std::time::Duration`'s own `Add`.
impl Add for Duration {
    type Output = Duration;

    fn add(self, rhs: Self) -> Self::Output {
        Duration(self.0 + rhs.0)
    }
}

/// `rel - rel = rel`, via `std::time::Duration`'s own `Sub`.
impl Sub for Duration {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration(self.0 - rhs.0)
    }
}

/// `n * rel = rel`, via `std::time::Duration`'s own `Mul<u32>`.
impl Mul<u32> for Duration {
    type Output = Duration;

    fn mul(self, rhs: u32) -> Self::Output {
        Duration(self.0 * rhs)
    }
}

/// `rel * n = rel`.
impl Mul<Duration> for u32 {
    type Output = Duration;

    fn mul(self, rhs: Duration) -> Self::Output {
        Duration(rhs.0 * self)
    }
}

/// `abs + rel = abs` -- the same shape `std::time::SystemTime`'s own
/// `Add<Duration>` takes, via `std::time::Duration`'s own `Add`.
impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Self::Output {
        Instant(self.0 + rhs.0)
    }
}

/// `rel + abs = abs`.
impl Add<Instant> for Duration {
    type Output = Instant;

    fn add(self, rhs: Instant) -> Self::Output {
        Instant(rhs.0 + self.0)
    }
}

/// `abs - rel = abs`, mirroring `std::time::SystemTime`'s own
/// `Sub<Duration>`.
impl Sub<Duration> for Instant {
    type Output = Instant;

    fn sub(self, rhs: Duration) -> Self::Output {
        Instant(self.0 - rhs.0)
    }
}

/// `abs - abs = rel`: the one way to turn two instants back into a
/// span, mirroring `std::time::SystemTime::duration_since` (infallible
/// here, unlike `SystemTime`'s own, since this crate's `Instant` can't
/// go backwards the way a real wall clock can). Deliberately no
/// `abs + abs` or `n * abs` -- neither is ever meaningful, so neither
/// is implemented (see this module's own doc).
impl Sub for Instant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration(self.0 - rhs.0)
    }
}

impl Instant {
    /// Like `Instant::sub` (`abs - abs = rel`), but saturates to a
    /// zero `Duration` instead of panicking when `earlier` is actually
    /// later than `self`.
    pub fn saturating_sub(self, earlier: Instant) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }

    /// `abs - rel = abs`, but `None` rather than panicking when `span`
    /// would push the result before the epoch.
    pub fn checked_sub(self, span: Duration) -> Option<Instant> {
        self.0.checked_sub(span.0).map(Instant)
    }
}
