import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import * as tx from "@subbit-tx/tx";
import { WalletLabel, wallets } from "./dapp";
import * as dapp from "./dapp";
import { inOrder } from "./utils";

export type Distribution = Partial<Record<WalletLabel, bigint>>;

const ADA = 1000000n;

export async function sendFunds(
  l: lucid.LucidEvolution,
  from: WalletLabel,
  to: Distribution,
) {
  let w = wallets(l.config().network!);
  const distribution_: Record<lucid.Address, Record<lucid.Unit, bigint>> = {};
  Object.entries(to).forEach(([user, ada]) => {
    distribution_[w[user].address] = { lovelace: ada * ADA };
  });
  await dapp.sequence(
    l,
    from,
    [() => kio.txs.distribute.tx(l, distribution_)],
    "distribution",
  );
  return {};
}

export async function clearFunds(
  l: lucid.LucidEvolution,
  from: WalletLabel[],
  to: WalletLabel,
) {
  console.log("FROM", from, "TO", to);
  let w = wallets(l.config().network!);
  const funded = await Promise.all(
    from.map(async (user) => [
      user,
      kio.lucidExtras.sumUtxos(await l.utxosAt(w[user].address)).lovelace >
        2_000_000n,
    ]),
  ).then((res) =>
    res.filter(([user, pred]) => pred).map(([user, _]) => user as WalletLabel),
  );
  return await inOrder(
    funded.map(
      (user) => () =>
        dapp.sequence(
          l,
          user,
          [() => kio.txs.clear.tx(l, w[to].address)],
          `clear:${user}`,
          (txb) => kio.txFinish.withChangeAddress(l, txb, w[to].address),
        ),
    ),
  );
}
