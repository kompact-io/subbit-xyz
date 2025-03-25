import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";

export async function single(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
): Promise<lucid.TxBuilder> {
  if (!(subbit.state.kind == "Opened"))
    throw new Error("Subbit must be opened");
  const opened = subbit.state.value;
  const txb = l.newTx().readFrom([ref]);
  const now = BigInt(
    lucid.slotToUnixTime(l.config().network!, l.currentSlot() + 3),
  );
  return step(txb, subbit.utxo, opened, now, v.closeRed());
}

export function step(
  txb: lucid.TxBuilder,
  utxo: lucid.UTxO,
  opened: v.OpenedE,
  now: bigint,
  red?: v.Redeemer,
): lucid.TxBuilder {
  const ub = now + 300000n; // 5 mins
  const deadline = ub + opened.constants.closePeriod + 1001n;
  return txb
    .collectFrom([utxo], v.redSer(red ? red : "Batch"))
    .addSignerKey(opened.constants.consumer)
    .validTo(Number(ub))
    .pay.ToAddressWithData(
      utxo.address,
      v.inlinedClosed(utxo.address, opened.constants, opened.subbed, deadline),
      utxo.assets,
    );
}
