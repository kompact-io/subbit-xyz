use super::require::SubbitAccept;
use super::spec;

/// We adopt some x402 Jargon, to accommodate the wrapping.
pub type Payload = crate::Request;

/// The client's payment authorization: `accepted` echoes the
/// `accepts[]` entry being paid under, `payload` is the crate root's
/// own `Request`.
pub type Request = spec::PaymentPayload<SubbitAccept, Payload>;
