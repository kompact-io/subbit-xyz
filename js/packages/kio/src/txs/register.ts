import * as lucid from "@lucid-evolution/lucid";

/* Register only if not already registered.
 *
 * The setup still expects a tx to be output, hence it returns a blank output.
 *
 * */

export async function tx(
  l: lucid.LucidEvolution,
  rewardAddress: lucid.RewardAddress,
): Promise<lucid.TxBuilder> {
  const x = await l.delegationAt(rewardAddress);
  const t = l.newTx();
  if (x == undefined || x.poolId == null) t.registerStake(rewardAddress);
  return t;
}
