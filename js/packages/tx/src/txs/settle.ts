import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import { hashKey } from "../utils";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
  amt: bigint,
  sig: string,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Closed"))
    throw new Error("Subbit must be closed");
  const closed = subbit.state.value;
  const txb = l.newTx().readFrom([ref]);
  return step(txb, subbit.utxo, closed, amt, v.settleRed(amt, sig));
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  closed: v.ClosedE,
  amt: bigint,
  red?: v.Redeemer,
): lucid.TxBuilder {
  const subAmt = v.calcSub(
    amt,
    closed.subbed,
    closed.amt,
    closed.unit == "lovelace",
  );
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .addSignerKey(closed.constants.provider)
    .pay.ToAddressWithData(
      utxo.address,
      v.inlinedSettled(utxo.address, closed.constants.consumer),
      Object.fromEntries([[closed.unit, closed.amt - subAmt]]),
    );
}
