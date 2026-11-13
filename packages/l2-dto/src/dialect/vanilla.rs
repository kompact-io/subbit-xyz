//! Subbit's native dialect: every submodule here is a straight
//! re-export of a crate-root type, nothing wrapped.

mod terms;
pub use terms::Terms;

mod request;
pub use request::Request;

mod response;
pub use response::Response;
