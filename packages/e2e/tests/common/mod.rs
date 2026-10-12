use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use subbit_e2e::Server;

/// The real `subbit-server` binary. `env!("CARGO_BIN_EXE_subbit-server")`
/// doesn't reach across packages — it only resolves binaries belonging
/// to the *current* package — so this locates it the same way
/// `assert_cmd::cargo_bin` does: every workspace binary lands in the
/// one shared `target/<profile>/` directory regardless of which
/// package defines it, and that directory is a fixed number of steps
/// above this test binary's own `current_exe()`.
///
/// Requires `subbit-server` to already be built. `cargo test
/// --workspace` (or `-p subbit-e2e -p subbit-server`) does this as a
/// side effect; running `cargo test -p subbit-e2e` alone does not —
/// run `cargo build -p subbit-server` first in that case.
fn server_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let mut dir = std::env::current_exe().expect("current_exe");
        dir.pop(); // this test binary's own file name
        if dir.ends_with("deps") {
            dir.pop();
        }
        let bin = dir.join(format!("subbit-server{}", std::env::consts::EXE_SUFFIX));
        assert!(
            bin.exists(),
            "subbit-server binary not found at {}; run `cargo build -p subbit-server` \
             (or `cargo test --workspace`) before running subbit-e2e's tests on their own",
            bin.display()
        );
        bin
    })
}

/// A flat per-request cost, both dialects on, otherwise default.
pub fn config(cost: u64) -> subbit_server::Config {
    let mut config = subbit_server::Config::default();
    config.terms.catalog.default.cost = subbit_l2_dto::Cost::ByRequest(cost);
    config
}

pub async fn spawn(cost: u64) -> anyhow::Result<Server> {
    Server::spawn(server_bin(), config(cost)).await
}
