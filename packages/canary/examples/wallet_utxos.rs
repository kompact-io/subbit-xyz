//! Prints the balance/UTxOs of canary's main wallet or the derived claim
//! wallet. Testing aid only -- not part of the canary binary.
//!
//! `cargo run -p subbit-canary --example wallet_utxos -- [main|claim]`

use anyhow::Context;
use cardano_sdk::{NetworkId, SigningKey};
use floe_client::grpc::Client;
use subbit_wallet::{Embedded, LandingPolicy, Wallet, wait_for_landing_with};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "main".to_string());
    let l2_seed_hex = std::env::var("L2_SEED_HEX").context("L2_SEED_HEX must be set")?;
    let l2_seed: [u8; 32] = hex::decode(l2_seed_hex.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("L2_SEED_HEX must decode to 32 bytes"))?;

    let main_seed = || -> anyhow::Result<[u8; 32]> {
        let key_hex = std::env::var("WALLET_SKEY").context("WALLET_SKEY must be set")?;
        hex::decode(key_hex.trim())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("WALLET_SKEY must decode to 32 bytes"))
    };

    let seed: [u8; 32] = match which.as_str() {
        "main" | "fund-claim" => main_seed()?,
        "claim" => *blake3::hash(&[l2_seed.as_slice(), b"claim-fee-wallet"].concat()).as_bytes(),
        other => anyhow::bail!("expected `main`, `claim`, or `fund-claim`, got {other:?}"),
    };

    let endpoint = std::env::var("FLOE_ENDPOINT")
        .unwrap_or_else(|_| "https://grpc.floe.kompact.io".to_string());
    let network = NetworkId::from(cardano_sdk::Network::Preprod);

    if which == "fund-claim" {
        let lovelace: u64 = args
            .next()
            .unwrap_or_else(|| "100000000".to_string())
            .parse()
            .context("expected lovelace amount")?;
        let claim_seed =
            *blake3::hash(&[l2_seed.as_slice(), b"claim-fee-wallet"].concat()).as_bytes();
        let claim_address = Embedded::new(
            Client::connect(&endpoint).await?,
            SigningKey::from(claim_seed),
            network,
            None,
        )
        .change_address()
        .await?;
        let mut main = Embedded::new(
            Client::connect(&endpoint).await?,
            SigningKey::from(main_seed()?),
            network,
            None,
        );
        let hash =
            subbit_console::send_payment(&mut main, "preprod", claim_address, lovelace).await?;
        println!("fund-claim: submitted {hash} for {lovelace} lovelace; waiting for landing");
        wait_for_landing_with(&mut main, hash, tokio::time::sleep, LandingPolicy::PATIENT).await?;
        println!("fund-claim: landed");
        return Ok(());
    }

    let backend = Client::connect(&endpoint).await?;
    let mut wallet = Embedded::new(backend, SigningKey::from(seed), network, None);

    let address = wallet.change_address().await?;
    let utxos = wallet.utxos(None).await?.unwrap_or_default();
    let total: u64 = utxos.values().map(|o| o.value().lovelace()).sum();
    println!("{which} wallet: {address}");
    println!("utxos: {} total: {total} lovelace", utxos.len());
    for (input, output) in utxos {
        println!(
            "  {input:?} {} lovelace{}",
            output.value().lovelace(),
            if output.script().is_some() {
                " (script)"
            } else {
                ""
            }
        );
    }
    Ok(())
}
