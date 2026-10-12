//! End-to-end harness for `subbit-server`: spawns the real server
//! binary, populates it via the fake indexer (`subbit-index`'s own
//! `Client`/`Row`, the library behind the `mock-index` binary), and
//! drives both wire dialects over real HTTP. No real chain involved —
//! see `docs/adrs/x402.md` and `plan-testing.md` for why this sits
//! between the chain-free unit tests and a real-preprod tier: it
//! exercises the real `axum` router, a real `redb`-backed `Db`, and
//! real Ed25519 signatures, across a real process boundary, without
//! needing a funded wallet or network access.

pub mod harness;
pub mod indexer;
pub mod vanilla;
pub mod wallet;
pub mod x402;

pub use harness::Server;
pub use wallet::Wallet;
