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

#[cfg(any(feature = "vanilla", feature = "x402"))]
pub(crate) fn charge_outcome(
    charge: crate::catalog::Charge,
    path: &str,
) -> reqwest_middleware::Result<ChargeOutcome> {
    use crate::catalog::Charge;
    match charge {
        // `policy` had its say and still declined: a caller concern (a
        // different policy, or a middleware layered in front of this
        // one), never a panic.
        Charge::Reject => Err(reqwest_middleware::Error::Middleware(anyhow::anyhow!(
            "rejected: no cost known for {path} and the miss policy declined to proceed"
        ))),
        Charge::Unpriced => Ok(ChargeOutcome::Unpriced),
        Charge::Priced(cost) => Ok(ChargeOutcome::Priced(cost)),
    }
}
