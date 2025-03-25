import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Closed"))
    throw new Error("Subbit must be closed");
  const closed = subbit.state.value;
  const txb = l.newTx().readFrom([ref]);
  const now = BigInt(
    lucid.slotToUnixTime(l.config().network!, l.currentSlot() + 3),
  );
  return step(txb, subbit.utxo, closed, now, v.expireRed());
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  closed: v.ClosedE,
  now: bigint,
  red?: v.Redeemer,
): lucid.TxBuilder {
  const lb = now - 300000n; // 5 mins
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .validFrom(Number(lb))
    .addSignerKey(closed.constants.consumer);
}
