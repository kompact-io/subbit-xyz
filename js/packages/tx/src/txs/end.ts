import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Settled"))
    throw new Error("Subbit must be settled");
  const consumer = subbit.state.value.consumer;
  const txb = l.newTx().readFrom([ref]);
  return step(txb, subbit.utxo, consumer, v.endRed());
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  consumer: string,
  red?: v.Redeemer,
): lucid.TxBuilder {
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .addSignerKey(consumer);
}
