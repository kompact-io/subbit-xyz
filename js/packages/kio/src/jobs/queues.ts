import * as lucid from "@lucid-evolution/lucid";
import retry from "async-retry";
import * as env from "../env";
import * as wallets from "../wallets";
import { logger } from "./logger";
import { txFinish } from "../index";
import { TxBuilder } from "../txFinish.js";

export async function sequence(
  l: lucid.LucidEvolution,
  walletLabel: string,
  txbs: (() => TxBuilder)[],
  txLabel: string = "",
  finish?: (_: TxBuilder) => Promise<string>,
) {
  /// Set wallet
  if (walletLabel !== "")
    l.selectWallet.fromPrivateKey(
      wallets.privateKey(env.privateKey(walletLabel)).to_bech32(),
    );
  if (txLabel !== "") console.log({ txLabel });
  /// Set defaults
  const finish_ = finish ? finish : (txb: TxBuilder) => txFinish.simple(l, txb);
  return await sequenceInner(l, walletLabel, txbs, finish_);
}

async function sequenceInner(
  l: lucid.LucidEvolution,
  walletLabel: string,
  txbs: (() => TxBuilder)[],
  finish: (_: TxBuilder) => Promise<string>,
) {
  const txb = txbs.pop();
  if (txb == undefined) return;
  const p = l.config().provider!;
  if ("log" in p) {
    await Promise.resolve(txb()).then(finish);
  } else {
    const txHash = await retry(
      (bail: any) => {
        return Promise.resolve(txb()).then(finish);
      },
      { retries: 2 },
    );
    if (txHash == undefined) {
      console.error("Tx failure");
      return;
    }
  }
  return await sequenceInner(l, walletLabel, txbs, finish);
}
