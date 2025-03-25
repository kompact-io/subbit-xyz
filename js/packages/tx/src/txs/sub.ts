import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import { hashKey } from "../utils";
import { ADA_BUFFER } from "../dapp";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
  amt: bigint,
  sig: string,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Opened"))
    throw new Error("Subbit must be opened");
  const opened = subbit.state.value;
  const txb = l.newTx().readFrom([ref]);
  return step(txb, subbit.utxo, opened, amt, v.subRed(amt, sig));
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  opened: v.OpenedE,
  amt: bigint,
  red?: v.Redeemer,
): lucid.TxBuilder {
  const subAmt = v.calcSub(
    amt,
    opened.subbed,
    opened.amt,
    opened.unit == "lovelace",
  );
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .addSignerKey(opened.constants.provider)
    .pay.ToAddressWithData(
      utxo.address,
      v.inlinedOpened(utxo.address, opened.constants, opened.subbed + subAmt),
      Object.fromEntries([[opened.unit, opened.amt - subAmt]]),
    );
}
