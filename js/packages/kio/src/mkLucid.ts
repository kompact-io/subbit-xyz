import * as lucid from "@lucid-evolution/lucid";
import { wallets } from "./wallets";
import { blockfrostLabel, env } from "./env";

function defaultAccounts(): lucid.EmulatorAccount[] {
  return Object.values(wallets("Preprod")).map((wi) => ({
    address: wi.address,
    assets: { lovelace: 1_000_000_000n },
    seedPhrase: "",
    privateKey: "",
  }));
}

export async function mkLucidWithEmulator(
  initialAccounts?: lucid.EmulatorAccount[],
  protocolParams?: lucid.ProtocolParameters,
): Promise<lucid.LucidEvolution> {
  const network = "Preprod";
  const accounts =
    initialAccounts === undefined ? defaultAccounts() : initialAccounts;
  const provider = new lucid.Emulator(accounts, protocolParams);
  return await lucid.Lucid(provider, network);
}

function extractNetwork(s: string): lucid.Network {
  const name = s.slice(blockfrostLabel.length);
  if (name == "MAINNET") return "Mainnet";
  if (name == "PREPROD") return "Preprod";
  if (name == "PREVIEW") return "Preview";
  console.log("Custom network");
  return "Custom";
}

function blockfrostBaseUrl(network: lucid.Network): URL {
  if (network == "Custom") return new URL("http://0.0.0.0:9999");
  const subdomain: Record<lucid.Network, string> = {
    Mainnet: "cardano-mainnet",
    Preprod: "cardano-preprod",
    Preview: "cardano-preview",
    Custom: "",
  };
  return new URL(`https://${subdomain[network]}.blockfrost.io/api/v0`);
}

export async function mkLucidWithBlockfrost(): Promise<lucid.LucidEvolution> {
  const bfKeys = Object.entries(env).find(([k, _]) =>
    k.startsWith(blockfrostLabel),
  );
  if (bfKeys == undefined) throw new Error("No bf key found");
  const [name, bfKey] = bfKeys;
  const network = extractNetwork(name);
  const provider = new lucid.Blockfrost(
    blockfrostBaseUrl(network).toString(),
    bfKey,
  );
  return await lucid.Lucid(provider, network);
}
