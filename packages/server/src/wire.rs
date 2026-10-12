//! The shape a reverse proxy in front of `subbit-server` actually
//! needs to speak: a status, some headers, a body, nothing `axum`-
//! specific. Every `dialect::Dialect` builds one of these (its own
//! doc comment has the specifics of when/why); `main.rs`'s
//! `into_axum_response` is the only place one becomes a real
//! `axum::response::Response`. Fronting `Ctx` with anything other
//! than axum means replacing that one translation, nothing here or
//! in `dialect` changes.

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

#[derive(Debug, Clone)]
pub struct Wire {
    pub status: StatusCode,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Vec<u8>,
}

impl Wire {
    pub fn new(status: StatusCode, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }

    pub fn text(status: StatusCode, message: impl Into<String>) -> Self {
        Self::new(status, message.into().into_bytes())
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.push((name, value));
        self
    }

    /// What `Ctx` hands back when no dialect is configured at all:
    /// the one case with no dialect left to ask for its own shape.
    pub fn no_supported_dialect() -> Self {
        Self::text(StatusCode::BAD_REQUEST, "no supported dialect")
    }
}

/// Does `headers` carry a non-empty value for `name` at all? Shared by
/// every dialect's own `recognizes`, not dialect-specific itself.
pub fn has_header(headers: &HeaderMap, name: &str) -> bool {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.is_empty())
}

/// A dialect with no real "payment required" shape of its own (no
/// discovery/challenge step, just a header it expects) answers a
/// missing header with this; one with no real decode-error shape of
/// its own answers an unparseable header with the other. Shared text,
/// not a `Dialect` method, since neither needs anything dialect
/// specific beyond the header's own name.
pub fn missing_header_wire(name: &str) -> Wire {
    Wire::text(
        StatusCode::PAYMENT_REQUIRED,
        format!("missing {name} header"),
    )
}

pub fn invalid_header_wire(name: &str) -> Wire {
    Wire::text(StatusCode::BAD_REQUEST, format!("invalid {name} header"))
}
