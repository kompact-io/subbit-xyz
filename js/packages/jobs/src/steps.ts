import * as lucid from "@lucid-evolution/lucid";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";

const ADA = 1_000_000n;

export async function add(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.add.single(l, ref, subbit, 2n * ADA)],
    "add",
  );
}

export async function addFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.add.single(l, ref, subbit, -2n * ADA)],
    "addFail",
  );
}

export async function sub(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const sig = dapp.sign("iou0", constants.subbitId, 2n * ADA);

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, 2n * ADA, sig)],
    "sub",
  );
}

export async function subFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const sig = dapp.sign("iou0", constants.subbitId, 1n * ADA);

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.sub.single(l, ref, subbit, 2n * ADA, sig)],
    "subFail",
  );
}

export async function close(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.close.single(l, ref, subbit)],
    "close",
  );
}

export async function closeFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.open.tx(l, ref, constants, 10n * ADA)],
    "open",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer1",
    [() => tx.txs.close.single(l, ref, subbit)],
    "close",
  );
}

export async function settle(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Closed: [constants, 0n, 1n] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const sig = dapp.sign("iou0", constants.subbitId, 2n * ADA);

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.settle.single(l, ref, subbit, 2n * ADA, sig)],
    "settle",
  );
}

export async function settleFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Closed: [constants, 2n * ADA, 1n] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const sig = dapp.sign("iou0", constants.subbitId, 2n * ADA);

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "provider0",
    [() => tx.txs.settle.single(l, ref, subbit, 2n * ADA, sig)],
    "settle",
  );
}

export async function end(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Settled: [w.consumer0.vkh] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const subbit = await tx.validator
    .getSettledByConsumer(l, address, w.consumer0.vkh)
    .then((r) => r[0]);
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.end.single(l, ref, subbit)],
    "settle",
  );
}

export async function endFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Settled: [w.consumer0.vkh] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const subbit = await tx.validator
    .getSettledByConsumer(l, address, w.consumer0.vkh)
    .then((r) => r[0]);
  await dapp.sequence(
    l,
    "consumer1",
    [() => tx.txs.end.single(l, ref, subbit)],
    "settle",
  );
}

export async function expire(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 1001n,
  };

  const now = BigInt(
    lucid.slotToUnixTime(l.config().network!, l.currentSlot() + 3),
  );

  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Closed: [constants, 0n, now + 20000n] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.expire.single(l, ref, subbit)],
    "expire",
  );
}

export async function expireFail(l: lucid.LucidEvolution, ref: lucid.UTxO) {
  const hash = lucid.validatorToScriptHash(new tx.validator.Validator());
  const address = tx.validator.mkAddress(l.config().network!, hash);
  const w = dapp.wallets(l.config().network!);
  const constants: tx.types.Constants = {
    subbitId: "deadbeef",
    currency: "Ada",
    iouKey: w.iou0.vkey,
    consumer: w.consumer0.vkh,
    provider: w.provider0.vkh,
    closePeriod: 999n,
  };

  const now = BigInt(
    lucid.slotToUnixTime(l.config().network!, l.currentSlot() + 3),
  );

  await dapp.sequence(
    l,
    "consumer0",
    [
      () =>
        tx.txs.fauxSubbit.tx(
          l,
          address,
          { Closed: [constants, 0n, now + 100000n] },
          { lovelace: 10n * ADA },
        ),
    ],
    "fauxSubbit",
  );

  const subbit = await tx.validator.getStateBySubbitId(
    l,
    address,
    constants.subbitId,
  );
  await dapp.sequence(
    l,
    "consumer0",
    [() => tx.txs.expire.single(l, ref, subbit)],
    "expire",
  );
}
