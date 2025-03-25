import * as lucid from "@lucid-evolution/lucid";
import { sumUtxos } from "../lucidExtras";

export async function tx(
  l: lucid.LucidEvolution,
  target: string,
): Promise<lucid.TxBuilder> {
  let allUtxos = await l.wallet().getUtxos();
  let sum = sumUtxos(allUtxos);
  sum["lovelace"] = sum["lovelace"] - 4_000_000n;
  const tx = l.newTx().collectFrom(allUtxos).pay.ToAddress(target, sum);
  return tx;
}
