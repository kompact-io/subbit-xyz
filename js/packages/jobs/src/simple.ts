import * as lucid from "@lucid-evolution/lucid";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";

const ADA = 1_000_000n;

export async function job(l: lucid.LucidEvolution) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const ref = await dapp.getRef(l, hash);
  const w = dapp.wallets(l.config().network!);

  const constants: tx.types.Constants = {
    subbitId: "1234567890abcdef1234567890abcdef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };
  const getSubbit = async () =>
    await tx.validator.getStateBySubbitId(l, address, constants.subbitId);
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  let iouAmt = 1n * ADA;
  let sig = dapp.sign("iou0", constants.subbitId, iouAmt);
  let subbit = await getSubbit();
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, iouAmt, sig)],
    "sub",
  );

  iouAmt = iouAmt + 2n * ADA;
  sig = dapp.sign("iou0", constants.subbitId, iouAmt);
  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, iouAmt, sig)],
    "sub",
  );

  iouAmt = iouAmt + 3n * ADA;
  sig = dapp.sign("iou0", constants.subbitId, iouAmt);
  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, iouAmt, sig)],
    "sub",
  );

  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.add.single(l, ref, subbit, 10n * ADA)],
    "add",
  );

  iouAmt = iouAmt + 4n * ADA;
  sig = dapp.sign("iou0", constants.subbitId, iouAmt);
  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, iouAmt, sig)],
    "sub",
  );

  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.close.single(l, ref, subbit)],
    "close",
  );

  iouAmt = iouAmt + 5n * ADA;
  sig = dapp.sign("iou0", constants.subbitId, iouAmt);
  subbit = await getSubbit();
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.settle.single(l, ref, subbit, iouAmt, sig)],
    "settle",
  );

  subbit = await tx.validator
    .getSettledByConsumer(l, address, constants.consumer)
    .then((r) => r[0]);
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.end.single(l, ref, subbit)],
    "end",
  );
}
