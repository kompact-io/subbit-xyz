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

export function privateKey(s: string) {
  if (s.startsWith("ed25519_sk1")) {
    return cml.PrivateKey.from_bech32(s);
  } else if (s.length == 64) {
    return cml.PrivateKey.from_normal_bytes(Buffer.from(s, "hex"));
  }
  throw new Error("Cannot coerce string to private key");
}

export const privateKeys = getPrivateKeys();

export function walletInfo(
  network: lucid.Network,
  id: string,
  sk: string,
): WalletInfo {
  const networkId = network === "Mainnet" ? 1 : 0;
  const addressPrefix = network === "Mainnet" ? "addr" : "addr_test";
  const skey = sk.startsWith("ed25519_sk1")
    ? cml.PrivateKey.from_bech32(sk)
    : cml.PrivateKey.from_normal_bytes(Buffer.from(sk, "hex"));
  const vkey = skey.to_public();
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
  // Guess form
  const sk = getPrivateKeys()[walletName]!;
  if (sk.startsWith("ed25519_sk1")) {
    l.selectWallet.fromPrivateKey(sk);
  } else {
    l.selectWallet.fromPrivateKey(
      cml.PrivateKey.from_normal_bytes(Buffer.from(sk, "hex")).to_bech32(),
    );
  }
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
