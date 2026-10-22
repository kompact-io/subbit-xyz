//! # Roles and responsibilities
//!
//! The service reports Backing of Channels and of TxOs that are _safe to settle_.
//!
//! Assumption: the service is treated as stateless, although may cache data as an optimization.
//!
//! A Backing corresponds to a TxO.
//!
//! The service verifies that a TxO is well-formed.
//!
//! - payment credential
//! - value is ada + maybe currency
//! - datum
//!     - own hash
//!     - constants: provider hash, long enough close period, currency
//!     - stage: well formed
//! - no script
//!
//! There are two ways the index report can be wrong about a Backing:
//!
//! - Reporting it exists when it doesn't
//! - Reporting it doesn't exist when it does
//!
//! The former is very bad - Provider is exposed to losing funds.
//! The latter is bad - Consumer does not receive service, but no funds at risk.
//!
//!
//! Assumption: The data source is able to fully resolve utxo inputs.
//! Thus from a tx we can establish "continuing" output from input-output ordering.
//!
//! Index hands over lineage: Server decides which to choose.
//! All other lines are "safe to settle".
//!
//! For
//!
//! Cases we must accommodate:
//!
//! - spends on a UTxO
//! - rollbacks
//! - mimics
//!
//! What happens if the service reports:
//!
//! - a safe to settle,
//! - then there is a rollback
//!
//! Either the Tx appears again or it does not. On the former case, it will be safe to settle.
//! In the latter it is unsettle-able.
//!
//! A notion of continuity in the case of a mutual
//!
//! Service should report : Current backing, and identify Output References that are _safe to
//! settle_.
//! A TxO is _safe to settle_ if it is not on the lineage of the current Backing.
//!
//!
//! Payload = { <keytag> : { backing , safe_to_settle : [ txo_ref ] } }
//! backing = { reserve, subbed }
//!
//! What does safe to settle mean? And what can go wrong?
//!
//! Under normal operation, there is a tx consuming the channel, and outputing a Closed UTxO.
//! The correspondence is unambiguous.
//!
//!
//!
//! A tx creates relationships between Inputs and Outputs.
//! A parent (input) and child (output) relationship is established by:
//!
//! _"Did the validator identify the child as the continuing output of the parent?"_
//!
//! A lineage is a sequence of parent-child relationships of TxOs.
//!
//! What is a transparent and robust way to establish relationships?
//! Rerunning the validator code?
//!
//! For each keytag, there is a vec of lineages.
//! In normal operation, this vec has one element,
//! but we must accommodate the case of more than one.
//!
//! If a lineage has an element in closed stage, then it is closed.
//! Otherwise is is open.
//!
//! TxO is **settled** if it is beyond a certain blocked depth `D`.
//! The depth `D` is at providers discression.
//! The backing of lineage is decided by

const PLACEHOLDER: u8 = 0;
