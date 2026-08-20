use std::path::{Path, PathBuf};

use clap::Parser;
use url::Url;

use subbit_index::client::Client;
use subbit_index::wire::{Row, parse_row, parse_rows};

/// Mock client for pushing (keytag -> backing) rows to the server.
#[derive(Parser)]
struct Args {
    /// File of rows, one per line (no header unless --header is set)
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long, requires = "file")]
    header: bool,
    /// Inline row, same format as a file line. Repeatable.
    #[arg(long = "row")]
    rows: Vec<String>,
    /// Base server URL, e.g. http://127.0.0.1:7822/v1/b
    #[arg(long, default_value = "http://127.0.0.1:7822/v1/a/backings", value_parser = Url::parse)]
    endpoint: Url,
}

fn load_file(path: &Path, header: bool) -> anyhow::Result<Vec<Row>> {
    Ok(parse_rows(&std::fs::read(path)?, header)?)
}

impl Args {
    fn collect(&self) -> anyhow::Result<Vec<Row>> {
        let mut rows = Vec::new();
        if let Some(path) = &self.file {
            rows.extend(load_file(path, self.header)?);
        }
        for r in &self.rows {
            rows.push(parse_row(r)?);
        }
        Ok(rows)
    }

    async fn run(self) -> anyhow::Result<()> {
        if self.file.is_none() && self.rows.is_empty() {
            anyhow::bail!("pass --file <path> and/or --row \"hex,amount,subbed\"");
        }
        let rows = self.collect()?;
        let result = Client::new(self.endpoint.clone()).send(rows).await?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    Args::parse().run().await
}
