import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
  amt: bigint,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Opened"))
    throw new Error("Subbit must be opened");
  const opened = subbit.state.value;
  const txb = l.newTx().readFrom([ref]);
  return step(txb, subbit.utxo, opened, amt, v.addRed());
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  opened: v.OpenedE,
  amt: bigint,
  red?: v.Redeemer,
): lucid.TxBuilder {
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .addSignerKey(opened.constants.consumer)
    .pay.ToAddressWithData(
      utxo.address,
      v.inlinedOpened(utxo.address, opened.constants, opened.subbed),
      Object.fromEntries([[opened.unit, opened.amt + amt]]),
    );
}
