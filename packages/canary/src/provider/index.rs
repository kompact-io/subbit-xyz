//! Spawns the `lineage-index` binary.

use tokio::process::Child;

use crate::process::spawn_with_config;

pub(crate) struct Process {
    _child: Child,
    _dir: tempfile::TempDir,
}

impl Process {
    pub(crate) async fn spawn(config: subbit_index::Config) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let child = spawn_with_config(&dir, "lineage-index", &config, &["run"])?;
        Ok(Self {
            _child: child,
            _dir: dir,
        })
    }
}
