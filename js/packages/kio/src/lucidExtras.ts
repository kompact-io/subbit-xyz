import * as lucid from "@lucid-evolution/lucid";

const SCRIPT_HASH_LENGTH = 56;
// FIXME : This must be available from lucid, but I can't find it.
export function mergeAssets(a: lucid.Assets, b: lucid.Assets) {
  const c = a;
  for (const [key, value] of Object.entries(b)) {
    if (Object.keys(c).includes(key)) {
      c[key] = c[key] + value;
    } else {
      c[key] = value;
    }
  }
  return c;
}

export function sumUtxos(utxos: lucid.UTxO[]): lucid.Assets {
  return utxos.map((u) => u.assets).reduce(mergeAssets, { lovelace: 0n });
}

export function toUnit(pid: string, name: string) {
  return `${pid}${name}`;
}

export function fromUnit(u: string): [string, string] {
  return [u.slice(0, SCRIPT_HASH_LENGTH), u.slice(SCRIPT_HASH_LENGTH)];
}

export type Ratio = {
  num: bigint;
  denom: bigint;
};

export function toRatio(num: bigint, denom: bigint): Ratio {
  return { num, denom };
}

export function fromRatio(r: Ratio): [bigint, bigint] {
  return [r.num, r.denom];
}

export async function pickUtxo(
  l: lucid.LucidEvolution,
  addr: string,
): Promise<lucid.UTxO> {
  let u = (await l.utxosAt(addr))[0];
  if (u === undefined) throw new Error("No utxos found");
  return u;
}

/// https://github.com/GoogleChromeLabs/jsbi/issues/30#issuecomment-1006088574
// eslint-disable-next-line @typescript-eslint/no-redeclare
interface BigInt {
  /** Convert to BigInt to string form in JSON.stringify */
  toJSON: () => string;
}

// @ts-ignore
BigInt.prototype.toJSON = function () {
  return this.toString();
};
