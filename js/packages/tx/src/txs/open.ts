import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import * as t from "../types";
import { maybeGetName } from "../utils";

export async function tx(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  constants: t.Constants,
  amt: bigint,
): Promise<lucid.TxBuilder> {
  const ownHash = lucid.validatorToScriptHash(ref.scriptRef!);
  const address = v.mkAddress(l.config().network!, ownHash);
  const name = await maybeGetName(l, constants.currency);
  return l
    .newTx()
    .pay.ToAddressWithData(
      address,
      v.inlined(v.opened(ownHash, constants, 0n)),
      v.assets(constants.currency, amt, name),
    );
}

export type OpenArgs = { constants: t.Constants; amt: bigint };

export async function many(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  arg: OpenArgs[],
): Promise<lucid.TxBuilder> {
  const ownHash = lucid.validatorToScriptHash(ref.scriptRef!);
  const address = v.mkAddress(l.config().network!, ownHash);
  const txb = l.newTx();
  arg.forEach(async ({ constants, amt }) => {
    const name = await maybeGetName(l, constants.currency);
    txb.pay.ToAddressWithData(
      address,
      v.inlined(v.opened(ownHash, constants, 0n)),
      v.assets(constants.currency, amt, name),
    );
  });
  return txb;
}
