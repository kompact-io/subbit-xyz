use cardano_connector_direct::Blockfrost;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Config {
    // ProjectKey
    Blockfrost { project_id: String },
}

impl Default for Config {
    fn default() -> Self {
        Self::Blockfrost {
            project_id: "mainnetxxxxxxxxxxxxxxxxxxxx".to_string(),
        }
    }
}

impl Config {
    pub fn build(self) -> Blockfrost {
        match self {
            Config::Blockfrost { project_id } => Blockfrost::new(project_id),
        }
    }
}
