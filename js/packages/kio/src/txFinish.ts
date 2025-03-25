import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";

export type TxBuilder = lucid.TxBuilder | Promise<lucid.TxBuilder>;

function explorerSubdomain(n: lucid.Network) {
  if (n === "Preview") return "preview.";
  if (n === "Preprod") return "preprod.";
  return "";
}

function explorerLink(l: lucid.LucidEvolution, txHash: string) {
  const c = l.config();
  if ("log" in c.provider!) return `emulator:${txHash}`;
  return `https://${explorerSubdomain(c.network!)}cexplorer.io/tx/${txHash}`;
}

export async function simple(l: lucid.LucidEvolution, txb: TxBuilder) {
  const txHash = await Promise.resolve(txb)
    .then((res) => res.complete())
    .then((res) => res.sign.withWallet())
    .then((res) => res.complete())
    .then((res) => res.submit());
  console.log(explorerLink(l, txHash));
  const _res = await l.awaitTx(txHash);
  return txHash;
}

export async function multisig(
  l: lucid.LucidEvolution,
  txb: TxBuilder,
  skeys: string[],
) {
  const txHash = await Promise.resolve(txb)
    .then((res) => res.complete())
    .then((res) => {
      skeys.forEach((skey) => res.sign.withPrivateKey(skey));
      return res;
    })
    .then((res) => res.complete())
    .then((res) => res.submit());
  console.log(explorerLink(l, txHash));
  const _res = await l.awaitTx(txHash);
  return txHash;
}

export async function withChangeAddress(
  l: lucid.LucidEvolution,
  txb: TxBuilder,
  address: string,
) {
  const txHash = await Promise.resolve(txb)
    .then((res) => res.complete({ changeAddress: address }))
    .then((res) => res.sign.withWallet())
    .then((res) => res.complete())
    .then((res) => res.submit());
  console.log(explorerLink(l, txHash));
  const _res = await l.awaitTx(txHash);
  return txHash;
}

export async function simpleNoUplcEval(
  l: lucid.LucidEvolution,
  txb: TxBuilder,
) {
  const txHash = await Promise.resolve(txb)
    .then((res) => res.complete({ localUPLCEval: false }))
    .then((res) => res.sign.withWallet())
    .then((res) => res.complete())
    .then((res) => res.submit());
  console.log(explorerLink(l, txHash));
  const _res = await l.awaitTx(txHash);
  return txHash;
}

export function iterateRedeemer(x: cml.Redeemers): cml.LegacyRedeemer[] {
  const x_ = x.as_arr_legacy_redeemer();
  if (x_ == undefined) throw new Error("Legacy redeemer fail");
  const l = x_.len();
  return [...Array(l).keys()].map((idx) => x_.get(idx));
}

export function iterateOutputs(
  x: cml.TransactionOutputList,
): cml.TransactionOutput[] {
  return [...Array(x.len()).keys()].map((idx) => x.get(idx));
}

export function iterateInputs(
  x: cml.TransactionInputList,
): cml.TransactionInput[] {
  return [...Array(x.len()).keys()].map((idx) => x.get(idx));
}

export async function costBreakdown(txb: TxBuilder) {
  const ctx = await Promise.resolve(txb)
    .then((res) => res.complete())
    .then((res) => res.sign.withWallet())
    .then((res) => res.complete());
  const tx = ctx.toTransaction();
  const deets = {
    _: tx.to_cbor_bytes().length,
    witness: {
      _: tx.witness_set().to_cbor_bytes().length,
      redeemers: {
        _: tx.witness_set().redeemers()?.to_cbor_bytes().length || 0,
        ...(tx.witness_set().redeemers() && {
          items: iterateRedeemer(tx.witness_set().redeemers()!).map(
            (x) => x.to_cbor_bytes().length,
          ),
        }),
      },
    },
    body: {
      _: tx.body().to_cbor_bytes().length,
      inputs: {
        _: "??",
        items: iterateInputs(tx.body().inputs()).map(
          (x) => x.to_cbor_bytes().length,
        ),
      },
      outputs: {
        _: "??",
        items: iterateOutputs(tx.body().outputs()).map((x) => ({
          _: x.to_cbor_bytes().length,
          amount: x.amount().to_cbor_bytes().length,
          address: x.address().to_raw_bytes().length,
          ...(x.datum() && { datum: x.datum()!.to_cbor_bytes().length }),
        })),
      },
    },
  };

  console.log("Deets");
  console.log(JSON.stringify(deets, null, 2));
  // const txHash = await ctx.submit()
  // console.log(explorerLink(l, txHash));
  // const _res = await l.awaitTx(txHash);
  //return txHash;
  return "notSubmitted";
}
