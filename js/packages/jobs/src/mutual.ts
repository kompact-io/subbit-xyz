import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";

const subbitId = "1234567890abcdef1234567890abcdef";

export async function setup(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId,
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 1n)],
    "open",
  );
}

export async function job(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const subbit = await tx.validator.getStateBySubbitId(l, address, subbitId);
  await kio.jobs.queues.sequence(
    l,
    "consumer0",
    [() => tx.txs.mutual.tx(l, ref, subbit)],
    "mutual",
    (txb) =>
      kio.txFinish.multisig(l, txb, [
        dapp.privateKey("provider0"),
        dapp.privateKey("consumer0"),
      ]),
  );
}

export async function oneSign(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const w = dapp.wallets(l.config().network!);
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const subbit = await tx.validator.getStateBySubbitId(l, address, subbitId);
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.mutual.txInner(l, ref, subbit, [w.consumer0.vkh])],
    "oneSign",
  );
}
