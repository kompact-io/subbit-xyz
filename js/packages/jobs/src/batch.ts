import * as lucid from "@lucid-evolution/lucid";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";

const ADA = 1_000_000n;

const mkTag = (idx: number) => String(idx).padStart(32, "0");

export async function job(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);

  const mkConstants = (userIdx: number, idx: number): tx.types.Constants => {
    const iouKey = userIdx == 0 ? w.iou0.vkey : w.iou1.vkey;
    const consumer = userIdx == 0 ? w.consumer0.vkey : w.consumer1.vkey;
    return {
      tag: mkTag(idx),
      currency: "Ada",
      iouKey,
      consumer,
      provider: w.provider0.vkh,
      closePeriod: 999n,
    };
  };

  await dapp.sequence(
    l,
    "consumer0",
    [...Array(10).keys()].map(
      (idx) => () => tx.txs.open.tx(l, ref, mkConstants(0, idx), 10n * ADA),
    ),
    "open",
  );

  await dapp.sequence(
    l,
    "consumer1",
    [...Array(9).keys()].map(
      (idx) => () =>
        tx.txs.open.tx(l, ref, mkConstants(0, idx + 10), 10n * ADA),
    ),
    "open",
  );

  const states = await tx.validator.getStates(l, address);
  const iouAmt = 1n * ADA;
  const sss: tx.validator.SubbitStep[] = states.map((ss) => {
    if (ss.state.kind != "Opened") throw new Error("Impossible");
    const iouLabel =
      ss.state.value.constants.iouKey == w.iou0.vkey ? "iou0" : "iou1";
    const sig = dapp.sign(iouLabel, ss.state.value.constants.tag, iouAmt);
    return {
      utxo: ss.utxo,
      state: ss.state.value,
      step: "sub",
      amt: iouAmt,
      sig: sig,
    };
  });
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.batch.tx(l, ref, sss)],
    "batch",
  );
}
