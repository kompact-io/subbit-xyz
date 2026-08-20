//! Local test client, no network I/O: `init` writes a starter config,
//! `status` prints it back with the current spend state, `request` builds
//! a request envelope for a URL path, `response` applies a base64
//! response envelope you got some other way.

use std::path::Path;

use clap::{Parser, Subcommand};
use subbit_issuer::native_url_lookup::{Config, NativeUrlLookup};

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    sources: subbit_config::Args,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter config to file.
    Init,
    /// Print the current status.
    Status,
    /// Build a request envelope for spending against a URL path.
    Request {
        path: String,
        /// Print as `{"subbit": "..."}` instead of raw base64.
        #[arg(long)]
        json: bool,
    },
    /// Apply a base64 response envelope.
    Response { body: String },
}

fn main() {
    let cli = Cli::parse();
    let sources = cli
        .sources
        .into_sources(Path::new("subbit-issuer-config.toml"));

    if let Cmd::Init = cli.cmd {
        let toml = toml::to_string_pretty(&Config::default()).expect("serialize config");
        std::fs::write(sources.base, toml).expect("write config");
        println!("wrote {}", sources.base.display());
        return;
    }

    let config: Config = sources.load().expect("failed to load config");

    match cli.cmd {
        Cmd::Init => unreachable!(),
        Cmd::Status => {
            let spender = NativeUrlLookup::build(&config).expect("build issuer");
            println!(
                "{}",
                serde_json::to_string_pretty(&spender.status()).expect("serialize status")
            );
        }
        Cmd::Request { path, json } => {
            let mut spender = NativeUrlLookup::build(&config).expect("build issuer");
            let envelope = spender.request(&path).expect("request envelope");
            if json {
                todo!("Not yet implemented")
                // println!("{}", serde_json::json!({ HEADER_NAME: envelope }));
            } else {
                println!("{envelope}");
            }
        }
        Cmd::Response { body } => {
            let mut spender = NativeUrlLookup::build(&config).expect("build issuer");
            match spender.respond(&body) {
                Ok(()) => println!("balance: {}", spender.issuer().spent()),
                Err(e) => eprintln!("error: {e}"),
            }
        }
    }
}
