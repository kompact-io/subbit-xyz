//! Wire types for the payload the index service reports to the subbit
//! server: see `subbit-index`'s README for the design this mirrors.

mod point;
pub use point::{OutputReference, Point};

mod line;
pub use line::{Item, Line, Stagish};

mod channel;
pub use channel::{Channel, ConstantsExt};

mod output;
pub use output::Output;
