import * as lucid from "@lucid-evolution/lucid";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";

export async function setup(l: lucid.LucidEvolution): Promise<lucid.UTxO> {
  const validator = new tx.validator.Validator();
  const hash = await dapp.putRef(l, validator);
  const ref = await dapp.getRef(l, hash);
  return ref;
}
