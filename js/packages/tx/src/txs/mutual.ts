import * as lucid from "@lucid-evolution/lucid";
import * as v from "../validator";
import * as t from "../types";
import { extractScriptHash } from "../utils";

/// Mutual txs can be more flexible than what is provided.
/// but this will cover most use cases.

export async function tx(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
  output?: {
    address: lucid.Address;
    stage: t.Stage;
    amt: bigint;
    name?: string;
  },
): Promise<lucid.TxBuilder> {
  if (subbit.state.kind == "Settled")
    throw new Error("Settled cannot be spent with Mutual");
  const consumer = subbit.state.value.constants.consumer;
  const provider = subbit.state.value.constants.provider;
  return txInner(l, ref, subbit, [consumer, provider], output);
}

/// For testing purposes only

export async function txInner(
  l: lucid.LucidEvolution,
  ref: lucid.UTxO,
  subbit: v.Subbit,
  signers: string[],
  output?: {
    address: lucid.Address;
    stage: t.Stage;
    amt: bigint;
    name?: string;
  },
): Promise<lucid.TxBuilder> {
  const txb = l
    .newTx()
    .readFrom([ref])
    .collectFrom([subbit.utxo], v.redSer("Mutual"));
  signers.forEach((s) => txb.addSignerKey(s));
  if (output == undefined) return txb;
  const { address, stage, amt, name } = output;
  if ("Settled" in output.stage)
    throw new Error("Just return funds to consumer directly");
  const ownHash = extractScriptHash(address);
  const constants = v.extractConstants(output);
  return txb.pay.ToAddressWithData(
    output.address,
    v.inlined({ ownHash, stage }),
    v.assets(constants.currency, amt, name),
  );
}
