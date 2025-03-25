import * as lucid from "@lucid-evolution/lucid";
import retry from "async-retry";

export async function getRef(
  l: lucid.LucidEvolution,
  addr: lucid.Address,
  tag: string,
): Promise<lucid.UTxO> {
  const dTag = lucid.Data.to<string>(lucid.fromText(tag));
  return await retry(
    async (_bail: any) => {
      const ref = (await l.utxosAt(addr))!.find(
        (u: lucid.UTxO) => u.datum == dTag && u.scriptRef,
      );
      if (ref === undefined) throw Error("ref undefined");
      return ref;
    },
    { retries: 1 },
  );
}

export async function getRefByHash(
  l: lucid.LucidEvolution,
  addr: lucid.Address,
  hash: string,
): Promise<lucid.UTxO> {
  return await retry(
    async (_bail: any) => {
      const ref = (await l.utxosAt(addr))!.find(
        (u) => u.scriptRef && lucid.validatorToScriptHash(u.scriptRef) == hash,
      );
      if (ref === undefined) throw Error("ref undefined");
      return ref;
    },
    { retries: 5 },
  );
}

export async function getRefs(
  l: lucid.LucidEvolution,
  addr: lucid.Address,
): Promise<Record<string, lucid.UTxO>> {
  return await retry(
    async (_bail: any) => {
      const refs = (await l.utxosAt(addr))!
        .filter((u: lucid.UTxO) => u.datum && u.scriptRef)
        .map((u) => [lucid.toText(lucid.Data.from<string>(u.datum!)), u]);
      return Object.fromEntries(refs);
    },
    { retries: 5 },
  );
}
