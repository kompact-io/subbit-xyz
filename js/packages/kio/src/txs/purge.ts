import * as lucid from "@lucid-evolution/lucid";
import { sumUtxos } from "../lucidExtras";

export async function tx(
  l: lucid.LucidEvolution,
  target: string | null = null,
): Promise<lucid.TxBuilder> {
  let target_ =
    target == null
      ? lucid.credentialToAddress(
          l.config().network!,
          lucid.keyHashToCredential("0".repeat(56)),
        )
      : target;
  let allUtxos = await l.wallet().getUtxos();
  let na = sumUtxos(allUtxos);
  na["lovelace"] = 0n;
  const tx = l.newTx().collectFrom(allUtxos).pay.ToAddress(target_, na);
  return tx;
}
