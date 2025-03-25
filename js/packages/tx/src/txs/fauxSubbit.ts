import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import * as t from "../types";
import { extractScriptHash } from "../utils";

/// Make an arbitrary subbit for testing

export async function tx(
  l: lucid.LucidEvolution,
  address: lucid.Address,
  stage: t.Stage,
  assets: lucid.Assets,
): Promise<lucid.TxBuilder> {
  const ownHash = extractScriptHash(address);
  const datum = { ownHash, stage };
  return l.newTx().pay.ToAddressWithData(address, v.inlined(datum), assets);
}
