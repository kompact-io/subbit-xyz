//! Native test-driving CLI for `subbit-console`'s core logic (plan-
//! subbit-console.md: "should have a native build for testing / test-
//! driving"). Not the shipped frontend -- that's `ui::wasm`, composed
//! into `packages/console/www`. This just exercises the same `Ctx` the
//! wasm bindings wrap, from a terminal, against a real file on disk.
//! `main.rs` is a one-line shim calling `run` here -- all the actual
//! argument parsing and dispatch lives in the library, not the binary,
//! so a future native TUI ("an interface very similar to the web-app")
//! has this shape to reuse rather than starting from `main.rs` again.
//!
//! `load`/`save` (below) are this adapter's own choice of *where* to
//! persist (a file) -- `Ctx` has no file-I/O code of its own at all;
//! see `ui::wasm`'s `load`/`save` for the same choice made differently
//! (`localStorage`).
//!
//! Narration goes through the same `crate::Logs` `ui::wasm` uses,
//! flushed to stderr once at the end of `run` -- stdout stays reserved
//! for each command's actual data output (a generated key's hex, a
//! `List`'s rows, `Export`'s JSON), same as any well-behaved CLI tool,
//! so only genuinely log-shaped narration (an action that otherwise
//! produces no output at all) goes through `Logs` here; a value this
//! binary has no other way to show stays a direct `println!` (see
//! `Logs`'s own doc on that split). Unlike `ui::wasm`, a failed action
//! here still aborts the whole process via `?` -- each invocation is
//! already exactly one action, so there's no live session to keep
//! going, and `main.rs`'s own `eprintln!("error: {e}")` already plays
//! the role `Logs` plays for the browser's longer-lived page.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use cardano_sdk::{Credential, Hash, Network, NetworkId};
use subbit_core::{Hash28, Tag, VerifyingKey};

use crate::{Ctx, Error, Level, Logs, ProviderEntry, SigningKeyHex};

fn load(path: &Path) -> Result<Ctx, Error> {
    if !path.exists() {
        return Ok(Ctx::default());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn save(ctx: &Ctx, path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(ctx)?)?;
    Ok(())
}

#[derive(Parser)]
struct Cli {
    #[arg(
        long,
        env = "SUBBIT_CONSOLE_PATH",
        default_value = "subbit-console.json"
    )]
    path: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(subcommand)]
    Keyring(KeyringCmd),
    #[command(subcommand)]
    Consumers(ConsumersCmd),
    #[command(subcommand)]
    Iou(IouCmd),
    #[command(subcommand)]
    Providers(ProvidersCmd),
    #[command(subcommand)]
    Basket(BasketCmd),
    #[command(subcommand)]
    Wallet(WalletCmd),
    /// Dump the whole store as JSON (what "Export" sends to a file).
    Export,
    /// Permanently clear the store at `--path`.
    Delete,
}

#[derive(Subcommand)]
enum KeyringCmd {
    /// Add an existing signing key (hex-encoded 32-byte ed25519 seed).
    Add {
        label: String,
        key: SigningKeyHex,
        /// Flags this key's derived vkh as usable for `Constants.consumer`.
        #[arg(long)]
        consumer: bool,
        /// Flags this key's verifying key as usable for `Constants.iou_key`.
        #[arg(long)]
        iou: bool,
    },
    /// Mint a fresh signing key and store it; prints the new key's hex
    /// seed once, for exporting into a `subbit-stamp` config.
    Generate {
        label: String,
        #[arg(long)]
        consumer: bool,
        #[arg(long)]
        iou: bool,
    },
    Remove {
        label: String,
    },
    List,
}

#[derive(Subcommand)]
enum ConsumersCmd {
    /// Add a verifying-only consumer vkh.
    Add {
        label: String,
        vkh: Hash28,
    },
    Remove {
        label: String,
    },
    /// Every consumer identity currently known, including ones derived
    /// from `Keyring` entries flagged `--consumer`.
    List,
}

#[derive(Subcommand)]
enum IouCmd {
    /// Add a verifying-only iou_key.
    Add {
        label: String,
        key: VerifyingKey,
    },
    Remove {
        label: String,
    },
    /// Every iou_key identity currently known, including ones derived
    /// from `Keyring` entries flagged `--iou`.
    List,
}

#[derive(Subcommand)]
enum ProvidersCmd {
    Add { label: String, base_url: String },
    Remove { label: String },
    List,
}

#[derive(Subcommand)]
enum BasketCmd {
    /// Stages an Open in `Ctx`'s own basket (persisted across
    /// invocations, like everything else) and saves it. Proves
    /// `crate::Basket` (plan section I) is usable from a native
    /// binary, not just from wasm, *and* that it now actually
    /// persists. Doesn't fetch from Floe or build a real transaction --
    /// those need a live `Backend`/wallet session this bin doesn't
    /// have yet.
    Open {
        tag: Tag,
        iou_key: VerifyingKey,
        consumer: Hash28,
        provider: Hash28,
        close_period_secs: u64,
        amount: u64,
    },
    /// Un-stages an Open by its hex keytag.
    DropOpen { keytag: String },
    /// Every Open currently staged.
    List,
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Mints a fresh key, stores it, and prints its hex seed once --
    /// save it now.
    Generate,
    /// Imports an existing raw hex signing key.
    Import {
        key_hex: String,
    },
    SetDelegation {
        credential_hex: Hash28,
    },
    ClearDelegation,
    /// Clears both the key and the delegation.
    Scrub,
    /// The held signing key's raw hex seed -- prints nothing if none
    /// is configured.
    Reveal,
    /// This wallet's address on `network` (`preprod`/`mainnet`).
    Address {
        network: String,
    },
    View,
}

pub fn run() -> Result<(), Error> {
    let cli = Cli::parse();
    let mut ctx = load(&cli.path)?;
    let mut logs = Logs::default();
    match cli.cmd {
        Cmd::Keyring(cmd) => match cmd {
            KeyringCmd::Add {
                label,
                key,
                consumer,
                iou,
            } => {
                ctx.keyring_add(label, key, consumer, iou)?;
                save(&ctx, &cli.path)?;
            }
            KeyringCmd::Generate {
                label,
                consumer,
                iou,
            } => {
                let key = ctx.keyring_generate(label.clone(), consumer, iou)?;
                save(&ctx, &cli.path)?;
                println!("{}", hex::encode(key.to_bytes()));
                logs.info(format!("generated key under label {label}"));
            }
            KeyringCmd::Remove { label } => {
                let removed = ctx.keyring_remove(&label);
                save(&ctx, &cli.path)?;
                println!("{removed}");
            }
            KeyringCmd::List => {
                for label in ctx.keyring_labels() {
                    println!("{label}");
                }
            }
        },
        Cmd::Consumers(cmd) => match cmd {
            ConsumersCmd::Add { label, vkh } => {
                ctx.consumers_add(label, vkh)?;
                save(&ctx, &cli.path)?;
            }
            ConsumersCmd::Remove { label } => {
                let removed = ctx.consumers_remove(&label);
                save(&ctx, &cli.path)?;
                println!("{removed}");
            }
            ConsumersCmd::List => {
                for (label, vkh) in ctx.consumers_effective() {
                    println!("{label}\t{vkh}");
                }
            }
        },
        Cmd::Iou(cmd) => match cmd {
            IouCmd::Add { label, key } => {
                ctx.iou_add(label, key)?;
                save(&ctx, &cli.path)?;
            }
            IouCmd::Remove { label } => {
                let removed = ctx.iou_remove(&label);
                save(&ctx, &cli.path)?;
                println!("{removed}");
            }
            IouCmd::List => {
                for (label, key) in ctx.iou_effective() {
                    println!("{label}\t{key}");
                }
            }
        },
        Cmd::Providers(cmd) => match cmd {
            ProvidersCmd::Add { label, base_url } => {
                // No fetch here -- a real Terms fetch is the frontend's
                // job (plain `fetch` in the browser, section D). The
                // native test-driving path only exercises storage.
                ctx.providers_insert(
                    label.clone(),
                    ProviderEntry {
                        base_url,
                        terms: None,
                        fetched_at: None,
                    },
                )?;
                save(&ctx, &cli.path)?;
                logs.info(format!("saved provider {label}"));
            }
            ProvidersCmd::Remove { label } => {
                let removed = ctx.providers_remove(&label);
                save(&ctx, &cli.path)?;
                println!("{removed}");
            }
            ProvidersCmd::List => {
                for (label, entry) in ctx.providers_list() {
                    println!("{label}\t{}", entry.base_url);
                }
            }
        },
        Cmd::Basket(cmd) => match cmd {
            BasketCmd::Open {
                tag,
                iou_key,
                consumer,
                provider,
                close_period_secs,
                amount,
            } => {
                let close_period = subbit_core::Duration::from_secs(close_period_secs);
                ctx.basket_add_open(
                    tag.clone(),
                    subbit_l1_dto::Asset::Lovelace,
                    iou_key,
                    consumer,
                    provider,
                    close_period,
                    None,
                    amount,
                );
                save(&ctx, &cli.path)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(ctx.basket_opens()).map_err(Error::from)?
                );
                logs.info(format!("staged open, tag={tag}"));
            }
            BasketCmd::DropOpen { keytag } => {
                let removed = ctx.basket_drop_open(&keytag);
                save(&ctx, &cli.path)?;
                println!("{removed}");
            }
            BasketCmd::List => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(ctx.basket_opens()).map_err(Error::from)?
                );
            }
        },
        Cmd::Wallet(cmd) => match cmd {
            WalletCmd::Generate => {
                let key = ctx.wallet_generate();
                save(&ctx, &cli.path)?;
                println!("{}", subbit_wallet::reveal_hex(&key));
            }
            WalletCmd::Import { key_hex } => {
                ctx.wallet_import(&key_hex)?;
                save(&ctx, &cli.path)?;
                logs.info("embedded wallet key imported");
            }
            WalletCmd::SetDelegation { credential_hex } => {
                let hash = Hash::<28>::from(<[u8; 28]>::from(credential_hex));
                ctx.wallet_set_delegation(Credential::from_key(hash));
                save(&ctx, &cli.path)?;
            }
            WalletCmd::ClearDelegation => {
                ctx.wallet_clear_delegation();
                save(&ctx, &cli.path)?;
            }
            WalletCmd::Scrub => {
                ctx.wallet_scrub();
                save(&ctx, &cli.path)?;
                logs.info("embedded wallet scrubbed");
            }
            WalletCmd::Reveal => {
                if let Some(key) = ctx.wallet_reveal() {
                    println!("{}", subbit_wallet::reveal_hex(&key));
                } else {
                    logs.info("reveal: no key configured");
                }
            }
            WalletCmd::Address { network } => {
                let network = NetworkId::from(
                    Network::try_from(network.as_str())
                        .map_err(|e| Error::Network(e.to_string()))?,
                );
                if let Some(address) = ctx.wallet_address(network) {
                    println!("{address}");
                }
            }
            WalletCmd::View => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ctx.wallet_view()).map_err(Error::from)?
                );
            }
        },
        Cmd::Export => {
            println!(
                "{}",
                serde_json::to_string_pretty(&ctx).map_err(Error::from)?
            );
        }
        Cmd::Delete => {
            ctx = Ctx::default();
            save(&ctx, &cli.path)?;
            logs.info("all data deleted");
        }
    }
    for entry in logs.entries() {
        match entry.level {
            Level::Info => eprintln!("[info] {}", entry.message),
            Level::Error => eprintln!("[error] {}", entry.message),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_roundtrips_and_missing_file_is_a_fresh_default() {
        let dir = std::env::temp_dir().join(format!("subbit-console-test-{}", std::process::id()));
        let path = dir.join("store.json");

        // No file yet -- `load` must hand back a fresh default, not error.
        let mut ctx = load(&path).unwrap();
        assert!(ctx.keyring_labels().is_empty());

        ctx.keyring_generate("alice".to_string(), false, false)
            .unwrap();
        save(&ctx, &path).unwrap();

        let restored = load(&path).unwrap();
        assert_eq!(restored.keyring_labels(), vec![&"alice".to_string()]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn basket_survives_a_json_file_roundtrip() {
        use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
        use subbit_l1_dto::Asset;

        let dir =
            std::env::temp_dir().join(format!("subbit-console-test-basket-{}", std::process::id()));
        let path = dir.join("store.json");

        let mut ctx = Ctx::default();
        ctx.basket_add_open(
            Tag::from(b"t".to_vec()),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
            Duration::from_secs(900),
            None,
            5_000_000,
        );
        save(&ctx, &path).unwrap();

        let restored = load(&path).unwrap();
        assert_eq!(restored.basket_opens().len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
