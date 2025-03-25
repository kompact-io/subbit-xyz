import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import * as add from "./add";
import * as sub from "./sub";
import * as close from "./close";
import * as settle from "./settle";
import * as end from "./end";
import * as expire from "./expire";

export async function tx(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbitSteps: v.SubbitStep[],
): Promise<lucid.TxBuilder> {
  subbitSteps.sort(v.compareSubbitStep);
  let mainRed = v.mkMain(subbitSteps);
  const mainStep = subbitSteps[0];
  if (mainStep == undefined) throw new Error("no steps given");
  const rest = subbitSteps.slice(1);
  const now = BigInt(
    lucid.slotToUnixTime(l.config().network!, l.currentSlot()),
  );
  const txb = l.newTx().readFrom([ref]);
  doStep(now, txb, mainStep, mainRed);
  rest.forEach((ss) => doStep(now, txb, ss, "Batch"));
  return txb;
}

function doStep(
  now: bigint,
  txb: lucid.TxBuilder,
  ss: v.SubbitStep,
  red: v.Redeemer,
) {
  if (ss.step == "add") {
    add.step(txb, ss.utxo, ss.state, ss.amt, red);
  } else if (ss.step == "sub") {
    sub.step(txb, ss.utxo, ss.state, ss.amt, red);
  } else if (ss.step == "close") {
    close.step(txb, ss.utxo, ss.state, now, red);
  } else if (ss.step == "settle") {
    settle.step(txb, ss.utxo, ss.state, ss.amt, red);
  } else if (ss.step == "end") {
    end.step(txb, ss.utxo, ss.state.consumer, red);
  } else if (ss.step == "expire") {
    expire.step(txb, ss.utxo, ss.state, now, red);
  } else {
    console.log("ISSUE WITH: \n", ss);
    throw new Error("Unknown step");
  }
}

// 16854 - 16507 = 347
// 16662
