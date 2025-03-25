import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import { walletLabel } from "@subbit-tx/kio/dist/src/env";
import * as tx from "@subbit-tx/tx";

/// Mainly a wrapping of `kio` with .env related things

export const walletLabels = [
  "admin",
  "store",
  "uploader",
  "provider0",
  "provider1",
  "consumer0",
  "iou0",
  "consumer1",
  "iou1",
] as const;
export type WalletLabel = (typeof walletLabels)[number];

export function wallets(
  n: lucid.Network,
): Record<WalletLabel, kio.wallets.WalletInfo> {
  let w = kio.wallets.wallets(n);
  return Object.fromEntries(
    walletLabels.map((x) => [x as WalletLabel, w[x]]),
  ) as Record<WalletLabel, kio.wallets.WalletInfo>;
}

export async function sequence(
  l: lucid.LucidEvolution,
  walletLabel: WalletLabel,
  txbs: (() => kio.txFinish.TxBuilder)[],
  txLabel: string = "",
  finish?: (_: kio.txFinish.TxBuilder) => Promise<string>,
) {
  if (finish) {
    return kio.jobs.queues.sequence(l, walletLabel, txbs, txLabel, finish);
  } else {
    return kio.jobs.queues.sequence(l, walletLabel, txbs, txLabel);
  }
}

export async function putRef(
  l: lucid.LucidEvolution,
  script?: lucid.Script,
  label?: string,
) {
  let w = wallets(l.config().network!);
  script = script || new tx.validator.Validator();
  await sequence(
    l,
    "uploader",
    [() => kio.txs.upload.tx(l, script, w.store.address, label || "")],
    `put:${label}`,
  );
  return lucid.validatorToScriptHash(script)!;
}

export async function getRef(l: lucid.LucidEvolution, hash?: string) {
  return kio.refs.getRefByHash(
    l,
    wallets(l.config().network!).store.address,
    hash || lucid.validatorToScriptHash(new tx.validator.Validator()),
  );
}

export function sign(w: WalletLabel, subbitId: string, amount: bigint): string {
  return tx.iou.sign(privateKey(w), { subbitId, amount });
}

export function verify(
  w: WalletLabel | string,
  subbitId: string,
  amount: bigint,
  sig: string,
): boolean {
  let vkey = walletLabels.includes(w as WalletLabel)
    ? wallets("Custom")[w].vkey
    : w;
  return tx.iou.verify(vkey, { subbitId, amount }, sig);
}

export function privateKey(w: WalletLabel): string {
  return kio.env.privateKey(w);
}
