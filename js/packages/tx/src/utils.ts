import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";
import * as kio from "@subbit-tx/kio";
import * as t from "./types";

export function isAda(c: t.Currency): boolean {
  return c == "Ada";
}

export function isByClass(c: t.Currency): boolean {
  return typeof c != "string" && "ByClass" in c;
}

export function isByHash(c: t.Currency): boolean {
  return typeof c != "string" && "ByHash" in c;
}

export function hashIfByHash(c: t.Currency): string | undefined {
  return typeof c != "string" && "ByHash" in c ? c.ByHash[0] : undefined;
}

/*
 * If using "ByHash" then we need to know the currency name.
 * We assume it must exist in the instantiated wallet,
 * since this is the only place we look.
 * FIME : use envvars?
 */
export async function getName(
  l: lucid.LucidEvolution,
  h: lucid.ScriptHash,
): Promise<string> {
  const assets = await l.wallet().getUtxos().then(kio.lucidExtras.sumUtxos);
  const unit = Object.keys(assets).find((u) => u.startsWith(h));
  if (unit == undefined) throw new Error(`No token found with hash ${h}`);
  return unit.slice(56);
}

/*
 * See `getName`
 */
export async function maybeGetName(
  l: lucid.LucidEvolution,
  c: t.Currency,
): Promise<string | undefined> {
  const hash = hashIfByHash(c);
  if (hash != undefined) {
    return await getName(l, hash);
  }
}

export function hashKey(vkey: string): string {
  return cml.PublicKey.from_bytes(lucid.fromHex(vkey)).hash().to_hex();
}

export function extractScriptHash(address: lucid.Address): lucid.ScriptHash {
  const cred = cml.Address.from_bech32(address).payment_cred();
  if (cred == undefined) throw new Error("bad input");
  const hash = cred.as_script();
  if (hash == undefined) throw new Error("not a script");
  return hash.to_hex();
}
