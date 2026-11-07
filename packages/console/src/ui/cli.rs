//! subbit-console via cli.
//!
//! Initially intended for testdriving logic. YMMV.
//!
//! Should operate just like the wasm.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

#[cfg(feature = "admin")]
use cardano_sdk::Input;
use cardano_sdk::{Credential, Hash, Network, NetworkId};
#[cfg(feature = "admin")]
use subbit_core::Signature;
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

/// Manages signing keys. Populates relevant known keys
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
    /// Remove a key fom keyring
    Remove { label: String },
    /// List all keys
    List,
}

/// Manage known Consumer VKH.
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

/// Manage known Verifying Keys
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

/// Manage known providers
#[derive(Subcommand)]
enum ProvidersCmd {
    Add { label: String, base_url: String },
    Remove { label: String },
    List,
}

/// Manage the "Basket" of steps you want commited in txs onchain.
#[derive(Subcommand)]
enum BasketCmd {
    /// Open a new channel
    Open {
        tag: Tag,
        // FIXME :: takes a raw key, not a stored label like `IouCmd::Add`'s `key` does.
        iou_key: VerifyingKey,
        // FIXME :: takes a raw hash, not a stored label like `ConsumersCmd::Add`'s `vkh` does.
        consumer: Hash28,
        provider: Hash28,
        close_period_secs: u64,
        amount: u64,
    },
    /// Un-stages an Open by its hex keytag.
    DropOpen { keytag: String },
    /// Every Open currently staged.
    List,
    /// Stage a `Sub` want directly against exactly one channel
    /// (ambiguous if `keytag` matches more than one -- pass `tx_oref`
    /// to disambiguate). No stage auto-detection -- see `Ctx::sub`.
    #[cfg(feature = "admin")]
    Sub {
        keytag: String,
        #[arg(long)]
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    },
    /// Stage a `Settle` want directly against exactly one channel
    /// (ambiguous if `keytag` matches more than one -- pass `tx_oref`
    /// to disambiguate). No stage auto-detection -- see `Ctx::settle`.
    #[cfg(feature = "admin")]
    Settle {
        keytag: String,
        #[arg(long)]
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    },
    /// Stage whichever of `Sub`/`Settle` is actually legal, per
    /// channel, for every channel matching `keytag` (or just
    /// `tx_oref` directly). See `Ctx::claim`'s own doc for what this
    /// does and doesn't do.
    #[cfg(feature = "admin")]
    Claim {
        keytag: String,
        /// Overrides the keytag lookup -- the only way to claim a
        /// channel the basket doesn't locally know about yet.
        #[arg(long)]
        tx_oref: Option<Input>,
        amount: u64,
        signature: Signature,
    },
    /// `Claim`'s batch sibling: stage many claims from a file of
    /// `keytag,tx_oref,amount,signature` lines (no header, no
    /// quoting) -- leave `tx_oref` empty to resolve it from `keytag`.
    #[cfg(feature = "admin")]
    ClaimMany { file: PathBuf },
}

/// Logs one `ClaimOutcome`, or an error, the same way for
/// `Sub`/`Settle`/`Claim`/`ClaimMany` -- makes clear this only
/// *stages*: a `basket build` + submit is still needed to actually
/// settle on-chain.
#[cfg(feature = "admin")]
fn log_outcome(logs: &mut Logs, keytag: &str, result: Result<crate::ClaimOutcome, Error>) {
    match result {
        Ok(outcome) => logs.info(format!(
            "staged {} for keytag={} tx_oref={} -- still needs `basket build` + submit to actually settle",
            outcome.step, outcome.keytag, outcome.tx_oref
        )),
        Err(e) => logs.error(format!("failed to stage claim for keytag={keytag}: {e}")),
    }
}

/// `BasketCmd::Claim`/`ClaimMany`'s shared plumbing: calls `Ctx::claim`
/// (which may stage more than one `Will`, e.g. across a reopened
/// channel's lineage) and logs every outcome.
#[cfg(feature = "admin")]
fn claim(
    ctx: &mut Ctx,
    logs: &mut Logs,
    keytag: &str,
    tx_oref: Option<Input>,
    amount: u64,
    signature: Signature,
) {
    for result in ctx.claim(keytag, tx_oref, amount, signature) {
        log_outcome(logs, keytag, result);
    }
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Mints a fresh key, stores it, and prints its hex seed once --
    /// save it now.
    Generate,
    /// Imports an existing raw hex signing key, read from stdin (one
    /// line) -- never as a CLI argument, which would leak it into shell
    /// history and `ps` output.
    Import,
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
            #[cfg(feature = "admin")]
            BasketCmd::Sub {
                keytag,
                tx_oref,
                amount,
                signature,
            } => {
                let result = ctx.sub(&keytag, tx_oref, amount, signature);
                log_outcome(&mut logs, &keytag, result);
                save(&ctx, &cli.path)?;
            }
            #[cfg(feature = "admin")]
            BasketCmd::Settle {
                keytag,
                tx_oref,
                amount,
                signature,
            } => {
                let result = ctx.settle(&keytag, tx_oref, amount, signature);
                log_outcome(&mut logs, &keytag, result);
                save(&ctx, &cli.path)?;
            }
            #[cfg(feature = "admin")]
            BasketCmd::Claim {
                keytag,
                tx_oref,
                amount,
                signature,
            } => {
                claim(&mut ctx, &mut logs, &keytag, tx_oref, amount, signature);
                save(&ctx, &cli.path)?;
            }
            #[cfg(feature = "admin")]
            BasketCmd::ClaimMany { file } => {
                let text = std::fs::read_to_string(&file)?;
                for (lineno, line) in text.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let fields: Vec<&str> = line.split(',').collect();
                    let [keytag, tx_oref, amount, signature] = fields[..] else {
                        logs.error(format!(
                            "{file}:{}: expected `keytag,tx_oref,amount,signature`, got {line:?}",
                            lineno + 1,
                            file = file.display()
                        ));
                        continue;
                    };
                    let tx_oref = if tx_oref.is_empty() {
                        Ok(None)
                    } else {
                        tx_oref.parse::<Input>().map(Some)
                    };
                    match (
                        tx_oref,
                        amount.parse::<u64>(),
                        signature.parse::<Signature>(),
                    ) {
                        (Ok(tx_oref), Ok(amount), Ok(signature)) => {
                            claim(&mut ctx, &mut logs, keytag, tx_oref, amount, signature)
                        }
                        _ => logs.error(format!(
                            "{}:{}: couldn't parse `{line}` as keytag,tx_oref,amount,signature",
                            file.display(),
                            lineno + 1
                        )),
                    }
                }
                save(&ctx, &cli.path)?;
            }
        },
        Cmd::Wallet(cmd) => match cmd {
            WalletCmd::Generate => {
                let key = ctx.wallet_generate();
                save(&ctx, &cli.path)?;
                println!("{}", subbit_wallet::reveal_hex(&key));
            }
            WalletCmd::Import => {
                let mut key_hex = String::new();
                std::io::stdin().read_line(&mut key_hex)?;
                ctx.wallet_import(key_hex.trim())?;
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
                } else {
                    logs.info("address: no wallet key configured");
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
