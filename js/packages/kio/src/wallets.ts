import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";
import { env, walletLabel } from "./env";

export interface WalletInfo {
  id: string;
  address: string;
  vkh: string;
  vkey: string;
}

export type Wallets = Record<string, WalletInfo>;

function getPrivateKeys(): Record<string, string> {
  return Object.fromEntries(
    Object.keys(env)
      .filter((k) => k.startsWith(walletLabel))
      .map((k) => [k.slice(walletLabel.length).toLowerCase(), env[k]]),
  );
}

export const privateKeys = getPrivateKeys();

export function walletInfo(
  network: lucid.Network,
  id: string,
  sk: string,
): WalletInfo {
  const networkId = network === "Mainnet" ? 1 : 0;
  const addressPrefix = network === "Mainnet" ? "addr" : "addr_test";
  const vkey = cml.PrivateKey.from_bech32(sk).to_public();
  const pkh = vkey.hash();
  const address = cml.EnterpriseAddress.new(
    networkId,
    cml.Credential.new_pub_key(pkh),
  );
  return {
    id,
    address: address.to_address().to_bech32(addressPrefix),
    vkey: lucid.toHex(vkey.to_raw_bytes()),
    vkh: pkh.to_hex(),
  };
}

export function setWallet(l: lucid.LucidEvolution, walletName: string) {
  l.selectWallet.fromPrivateKey(getPrivateKeys()[walletName]!);
  return l;
}

export function wallets(
  network: lucid.Network,
  sks?: Record<string, string>,
): Wallets {
  return Object.fromEntries(
    Object.entries(sks || getPrivateKeys()).map(([k, v]) => [
      k,
      walletInfo(network, k, v),
    ]),
  );
}
