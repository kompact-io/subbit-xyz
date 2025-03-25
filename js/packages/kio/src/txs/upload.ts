import * as lucid from "@lucid-evolution/lucid";

export async function tx(
  l: lucid.LucidEvolution,
  script: lucid.Script,
  hostAddress: string,
  tag: string,
): Promise<lucid.TxBuilder> {
  return l
    .newTx()
    .pay.ToAddressWithData(
      hostAddress,
      { kind: "inline", value: lucid.Data.to<string>(lucid.fromText(tag)) },
      { lovelace: 1n },
      script,
    );
}
