use std::path::Path;

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {} as JSON", path.display()))
}

/// Like `load`, but `Ok(None)` instead of an error when the file is
/// missing (e.g. before the first `tx new`).
pub fn try_load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(load(path)?))
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}
