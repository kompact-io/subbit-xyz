import * as lucid from "@lucid-evolution/lucid";

export type Distribute = Record<lucid.Address, Record<lucid.Unit, bigint>>;

export async function tx(
  l: lucid.LucidEvolution,
  d: Distribute,
): Promise<lucid.TxBuilder> {
  const tx = l.newTx();
  Object.entries(d).forEach(([addr, val]) => {
    tx.pay.ToAddress(addr, val);
  });
  return tx;
}
