import * as lucid from "@lucid-evolution/lucid";
import * as t from "./types";
import * as dapp from "./dapp";
import { SubbitSubbitSpend } from "./blueprint";
import { extractScriptHash } from "./utils";

export type Validator = SubbitSubbitSpend;
export const Validator = SubbitSubbitSpend;

const d: Datum = {
  ownHash: "",
  stage: {
    Opened: [
      {
        tag: "",
        currency: "Ada",
        iouKey: "",
        consumer: "",
        provider: "",
        closePeriod: 0n,
      },
      0n,
    ],
  },
};

export const label = "subbit";
export type Params = ConstructorParameters<Validator>;
export type Redeemer = Validator["redeemer"];
export const Redeemer = Validator["redeemer"];
export type Datum = Validator["datum"];
export const Datum = Validator["datum"];

export function redSer(x: Redeemer): string {
  return lucid.Data.to<Redeemer>(x, Redeemer);
}

export function redDeser(x: string): Redeemer {
  return lucid.Data.from<Redeemer>(x, Redeemer);
}

export function datSer(x: Datum): string {
  return lucid.Data.to<Datum>(x, Datum);
}

export function datDeser(x: string): Datum {
  return lucid.Data.from<Datum>(x, Datum);
}

export function mkAddress(
  network: lucid.Network,
  ownHash?: string,
  stakeCred?: lucid.Credential,
): lucid.Address {
  ownHash = ownHash || lucid.validatorToScriptHash(new Validator());
  const ownCred: lucid.Credential = { type: "Script", hash: ownHash };
  if (stakeCred) {
    return lucid.credentialToAddress(network, ownCred, stakeCred);
  }
  return lucid.credentialToAddress(network, ownCred);
}

export function inlined(d: Datum): lucid.OutputDatum {
  return { kind: "inline", value: datSer(d) };
}

export function toUnit(currency: t.Currency, name?: string): lucid.Unit {
  if (currency == "Ada") {
    return "lovelace";
  } else if ("ByHash" in currency) {
    const hash = currency.ByHash;
    return `${hash}${name || ""}`;
  } else {
    const [hash, name] = currency.ByClass;
    return `${hash}${name}`;
  }
}

export function guessNameFromAssets(
  hash: lucid.ScriptHash,
  assets: lucid.Assets,
): lucid.Unit {
  const unit = Object.keys(assets).find((u) => u.startsWith(hash));
  if (unit == undefined) throw new Error("No asset to guess name");
  return unit;
}

/// Guess the currency unit with available information.
/// if the `currency` is available (ie not `Settled`), use that,
/// else infer from assets.
export function guessUnit(
  currency: t.Currency,
  assets: lucid.Assets,
  name?: string,
): lucid.Unit {
  if (currency == "Ada") {
    return "lovelace";
  } else if ("ByHash" in currency) {
    const hash = currency.ByHash[0];
    const name_ = name != undefined ? name : guessNameFromAssets(hash, assets);
    return `${hash}${name_}`;
  } else {
    const [hash, name] = currency.ByClass[0];
    return `${hash}${name}`;
  }
}

export function assets(
  currency: t.Currency,
  amt: bigint,
  name?: string,
): lucid.Assets | undefined {
  return Object.fromEntries([[toUnit(currency, name), amt]]);
}

export function stage(ownHash: lucid.ScriptHash, stage: t.Stage): Datum {
  return { ownHash, stage };
}

export function opened(
  ownHash: lucid.ScriptHash,
  constants: t.Constants,
  amt: bigint,
): Datum {
  return stage(ownHash, { Opened: [constants, amt] });
}

export function closed(
  ownHash: lucid.ScriptHash,
  constants: t.Constants,
  amt: bigint,
  deadline: bigint,
): Datum {
  return stage(ownHash, { Closed: [constants, amt, deadline] });
}

export function settled(ownHash: lucid.ScriptHash, vkey: string): Datum {
  return stage(ownHash, { Settled: [vkey] });
}

export function inlinedStage(
  address: lucid.Address,
  stage: t.Stage,
): lucid.OutputDatum {
  return inlined({ ownHash: extractScriptHash(address), stage });
}

export function inlinedOpened(
  address: lucid.Address,
  constants: t.Constants,
  amt: bigint,
): lucid.OutputDatum {
  return inlinedStage(address, { Opened: [constants, amt] });
}

export function inlinedClosed(
  address: lucid.Address,
  constants: t.Constants,
  amt: bigint,
  deadline: bigint,
): lucid.OutputDatum {
  return inlinedStage(address, { Closed: [constants, amt, deadline] });
}

export function inlinedSettled(
  address: lucid.Address,
  vkey: string,
): lucid.OutputDatum {
  return inlinedStage(address, { Settled: [vkey] });
}

export function mainRed(steps: t.Steps): Redeemer {
  return { Main: [steps] };
}

export function contRed(step: t.Cont): t.Step {
  return { StepCont: [step] };
}

export function eolRed(step: t.Eol): t.Step {
  return { StepEol: [step] };
}

export function addRed(): Redeemer {
  return mainRed([contRed("Add")]);
}

export function subRed(amt: bigint, sig: string): Redeemer {
  return mainRed([contRed({ Sub: [{ amount: amt, signature: sig }] })]);
}

export function closeRed(): Redeemer {
  return mainRed([contRed("Close")]);
}

export function settleRed(amt: bigint, sig: string): Redeemer {
  return mainRed([contRed({ Settle: [{ amount: amt, signature: sig }] })]);
}

export function endRed(): Redeemer {
  return mainRed([eolRed("End")]);
}

export function expireRed(): Redeemer {
  return mainRed([eolRed("Expire")]);
}

export function concatMain(l: Redeemer, r: Redeemer): Redeemer {
  if (typeof l == "string" || !("Main" in l))
    throw new Error("Expected main Redeemer");
  if (typeof r == "string" || !("Main" in r))
    throw new Error("Expected main Redeemer");
  return mainRed([...l.Main[0], ...r.Main[0]]);
}

export type Subbit = {
  utxo: lucid.UTxO;
  state: State;
};

export type State =
  | { kind: "Opened"; value: OpenedE }
  | { kind: "Closed"; value: ClosedE }
  | { kind: "Settled"; value: SettledE };

export type OpenedE = {
  constants: t.Constants;
  subbed: bigint;
  unit: lucid.Unit;
  amt: bigint;
};
export type ClosedE = {
  constants: t.Constants;
  subbed: bigint;
  deadline: bigint;
  unit: lucid.Unit;
  amt: bigint;
};
export type SettledE = {
  consumer: string;
};

export function extractConstants(datum: { stage: t.Stage }): t.Constants {
  if ("Opened" in datum.stage) return datum.stage.Opened[0];
  if ("Closed" in datum.stage) return datum.stage.Closed[0];
  throw new Error("datum must contain constants");
}

export function coerceDatum(
  maybeDatum: string | undefined | null,
): Datum | undefined {
  if (maybeDatum == undefined || maybeDatum == null) {
    console.warn("Utxo has no datum");
    return;
  }
  const datum = datDeser(maybeDatum);
  if (datum == undefined) {
    console.warn("Cannot coerce datum");
    return;
  }
  return datum;
}

export function utxo2State(utxo: lucid.UTxO, name?: string): State | undefined {
  const datum = coerceDatum(utxo.datum);
  if (datum == undefined) return;
  if ("Settled" in datum.stage)
    return { kind: "Settled", value: { consumer: datum.stage.Settled[0] } };
  if ("Opened" in datum.stage) {
    const [constants, subbed] = datum.stage.Opened;
    const unit = guessUnit(constants.currency, utxo.assets, name);
    const amt = utxo.assets[unit] || 0n;
    return { kind: "Opened", value: { constants, subbed, unit, amt } };
  }
  if ("Closed" in datum.stage) {
    const [constants, subbed, deadline] = datum.stage.Closed;
    const unit = guessUnit(constants.currency, utxo.assets, name);
    const amt = utxo.assets[unit] || 0n;
    return {
      kind: "Closed",
      value: { constants, subbed, deadline, unit, amt },
    };
  }
}

export function utxo2Subbit(
  utxo: lucid.UTxO,
  name?: string,
): Subbit | undefined {
  const state = utxo2State(utxo, name);
  if (state == undefined) return;
  return { utxo, state };
}

export async function getStates(
  l: lucid.LucidEvolution,
  address: lucid.Address,
): Promise<Subbit[]> {
  const utxos = await l.utxosAt(address);
  const subbits: Subbit[] = [];
  utxos.forEach((utxo) => {
    const subbit = utxo2Subbit(utxo);
    if (subbit != undefined) subbits.push(subbit);
  });
  return subbits;
}

export async function getStateByTag(
  l: lucid.LucidEvolution,
  address: lucid.Address,
  tag: string,
): Promise<Subbit> {
  const candidates = await getStatesByTag(l, address, tag);
  const candidate = candidates.pop();
  if (candidate == undefined) throw new Error("No subbit found");
  if (candidates.length > 0)
    console.warn(`More than one subbit with given id: ${tag}`);
  return candidate;
}

export async function getStatesByTag(
  l: lucid.LucidEvolution,
  address: lucid.Address,
  tag: string,
): Promise<Subbit[]> {
  const subbits = await getStates(l, address);
  return subbits.filter((r) => {
    if (r.state.kind == "Settled") return false;
    return r.state.value.constants.tag == tag;
  });
}

export async function getSettledByConsumer(
  l: lucid.LucidEvolution,
  address: lucid.Address,
  consumer: string,
): Promise<Subbit[]> {
  return await getStates(l, address).then((res) =>
    res.filter((r) => {
      if (r.state.kind != "Settled") return false;
      return r.state.value.consumer == consumer;
    }),
  );
}

export type SubbitStep =
  | { utxo: lucid.UTxO; state: OpenedE; step: "add"; amt: bigint }
  | { utxo: lucid.UTxO; state: OpenedE; step: "sub"; amt: bigint; sig: string }
  | { utxo: lucid.UTxO; state: OpenedE; step: "close" }
  | {
      utxo: lucid.UTxO;
      state: ClosedE;
      step: "settle";
      amt: bigint;
      sig: string;
    }
  | { utxo: lucid.UTxO; state: ClosedE; step: "expire" }
  | { utxo: lucid.UTxO; state: SettledE; step: "end" };

export function compareUtxos(a: lucid.UTxO, b: lucid.UTxO) {
  if (a.txHash < b.txHash) return -1;
  if (a.txHash > b.txHash) return 1;
  return a.outputIndex < b.outputIndex ? -1 : 1;
}

export function compareSubbitStep(a: SubbitStep, b: SubbitStep) {
  return compareUtxos(a.utxo, b.utxo);
}

export function mkMain(sss: SubbitStep[]): Redeemer {
  sss.sort(compareSubbitStep);
  let red = mainRed([]);
  sss.forEach((ss) => {
    if (ss.step == "add") {
      red = concatMain(red, addRed());
    } else if (ss.step == "sub") {
      red = concatMain(red, subRed(ss.amt, ss.sig));
    } else if (ss.step == "close") {
      red = concatMain(red, closeRed());
    } else if (ss.step == "settle") {
      red = concatMain(red, settleRed(ss.amt, ss.sig));
    } else if (ss.step == "end") {
      red = concatMain(red, endRed());
    } else if (ss.step == "expire") {
      red = concatMain(red, expireRed());
    } else {
      console.log("ISSUE WITH: \n", ss);
      throw new Error("Unknown step");
    }
  });
  return red;
}

export function calcSub(
  next: bigint,
  curr: bigint,
  amt: bigint,
  isAda: boolean,
) {
  const diff = next - curr;
  if (diff <= 0n) throw new Error("Diff must be > 0");
  const available = amt - (isAda ? dapp.ADA_BUFFER : 0n);
  if (available <= 0n) throw new Error("No available funds");
  if (diff > available) console.warn("Insufficient funds available");
  return diff > available ? available : diff;
}
