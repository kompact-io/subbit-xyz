use matchit::Router;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, Default)]
pub struct Config {
    #[n(0)]
    default: u64,
    #[n(1)]
    rules: BTreeMap<String, u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("insert: {pattern}, {source:?}")]
    Insert {
        pattern: String,
        source: matchit::InsertError,
    },
}

pub struct Costings {
    router: Router<u64>,
    default: u64,
}

impl Costings {
    pub fn new(config: Config) -> Result<Self, Error> {
        let mut router = Router::new();
        for (pattern, cost) in config.rules {
            router
                .insert(&pattern, cost)
                .map_err(|source| Error::Insert { pattern, source })?;
        }

        Ok(Self {
            router,
            default: config.default,
        })
    }

    pub fn lookup(&self, path: &str) -> u64 {
        self.router
            .at(path)
            .map(|m| *m.value)
            .unwrap_or(self.default)
    }
}
