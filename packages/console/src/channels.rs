//! Logic about an observed on-chain channel. "Channel" here is
//! shorthand for a single on-chain txo decoding to one -- a channel's
//! real on-chain presence is a *line*, potentially many txos over its
//! lifetime via continuing outputs, with finality/depth nuances
//! `finality`/`status`/`reconcile` deliberately simplify away (each
//! operates on one observed snapshot, not a whole line): `reconcile`
//! (joining the txo's `Constants` against `crate::Providers`'s matched
//! `Terms` and reporting divergence), `finality` (whether a lineage
//! item has reached a matched Terms' `confirmation_depth` yet), and
//! `status` (whether a txo is this consumer's, and which actions it
//! currently permits). `lineage` is the other half -- the full-history
//! view, grouped by Constants/line (index-dto's own organization, kept
//! as-is -- see that module's doc on why), that *does* look at a whole
//! line at once. `brief` labels a Constants' keys/provider for the
//! widget `lineage` renders per Constants. Grouped under one `channels`
//! module -- the same name as the Channels tab/feature this code backs
//! -- since all five take an observed channel's own data as their
//! subject; see each submodule's own doc for plan-subbit-console.md's
//! sections F and J.

pub mod brief;
pub mod finality;
pub mod lineage;
pub mod reconcile;
pub mod status;

pub use brief::{Brief, ProviderMatch};
pub use finality::is_final;
pub use lineage::{ConstantsLines, LineView, LineageView};
pub use reconcile::{Divergence, Match, reconcile};
pub use status::{Status, statuses};
