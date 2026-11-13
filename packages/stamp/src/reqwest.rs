#[cfg(feature = "vanilla")]
pub mod vanilla;
#[cfg(feature = "x402")]
pub mod x402;

/// Shared by every reqwest middleware's `handle`: turns a resolved
/// [`crate::catalog::Charge`] into either the cost to envelope the
/// request with, or an early outcome (a caller-visible rejection, or
/// "send unmetered and skip straight to the reply") the caller
/// returns directly.
#[cfg(any(feature = "vanilla", feature = "x402"))]
pub(crate) enum ChargeOutcome {
    /// No payment envelope: forward the request as-is.
    Unpriced,
    /// Pay this amount.
    Priced(u64),
}

/// Typed errors this module can raise, kept separate from `anyhow` so
/// a caller inspecting `reqwest_middleware::Error::Middleware`'s
/// inner value (via `downcast_ref`) sees a real type -- `anyhow` is
/// only ever the wire this crosses into `reqwest_middleware`'s own
/// trait signature, never the error's native shape.
#[cfg(any(feature = "vanilla", feature = "x402"))]
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("rejected: no cost known for {path} and the miss policy declined to proceed")]
    Rejected { path: String },
}

#[cfg(any(feature = "vanilla", feature = "x402"))]
pub(crate) fn charge_outcome(
    charge: crate::catalog::Charge,
    path: &str,
) -> Result<ChargeOutcome, Error> {
    use crate::catalog::Charge;
    match charge {
        // `policy` had its say and still declined: a caller concern (a
        // different policy, or a middleware layered in front of this
        // one), never a panic.
        Charge::Reject => Err(Error::Rejected {
            path: path.to_string(),
        }),
        Charge::Unpriced => Ok(ChargeOutcome::Unpriced),
        Charge::Priced(cost) => Ok(ChargeOutcome::Priced(cost)),
    }
}
