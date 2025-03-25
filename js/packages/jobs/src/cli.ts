import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import * as tx from "@subbit-tx/tx";
import * as dapp from "./dapp";
import * as funds from "./funds";
import { Command } from "commander";

const ADA = 1000000n;

function main() {
  cmds().parse();
}

function cmds() {
  const cmd = new Command().name("subbit");
  const txCmd = cmd.command("tx").description("Txs cli");
  addTxs(txCmd);
  const showCmd = cmd.command("show").description("Show dapp info");
  addShows(showCmd);
  const iouCmd = cmd.command("iou").description("Handle ious");
  addIous(iouCmd);
  return cmd;
}

function addTxs(cmd: Command) {
  removeScripts(cmd);
  sendFunds(cmd);
  clearFunds(cmd);
  putRef(cmd);
  open(cmd);
  openMany(cmd);
  add(cmd);
  sub(cmd);
  close(cmd);
  settle(cmd);
  end(cmd);
  expire(cmd);
  batch(cmd);
  mutualEnd(cmd);
}

function addShows(cmd: Command) {
  showWallets(cmd);
  showSubbits(cmd);
}

function showWallets(cmd: Command) {
  cmd
    .command("wallets")
    .description("Show state of wallets")
    .action(async (_args, _ctx) => {
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      Object.entries(w).forEach(async ([key, val]) => {
        let tot = await l
          .utxosAt(val.address)
          .then((r) => kio.lucidExtras.sumUtxos(r));
        console.log(key, "\n", w[key].address, "\n", w[key].vkh, "\n", tot);
      });
    });
}

function hasConsumer(s: tx.validator.Subbit, consumer: string) {
  if (s.state.kind == "Settled") {
    return s.state.value.consumer == consumer;
  } else {
    return s.state.value.constants.consumer == consumer;
  }
}

function hasProvider(s: tx.validator.Subbit, provider: string) {
  if (s.state.kind == "Settled") {
    return false;
  } else {
    return s.state.value.constants.provider == provider;
  }
}

function showSubbits(cmd: Command) {
  cmd
    .command("subbits")
    .description("Show subbits")
    .option("--subbit-id <subbit-id>", "Filter on subbit id")
    .option("--consumer <consumer>", "Filter on consumer (wallet label or vkh)")
    .option("--provider <provider>", "Filter on provider (wallet label or vkh)")
    .option(
      "--vkh <vkh>",
      "Filter on vkh of either consumer or provider (wallet label or vkh)",
    )
    .option("--address <address>", "Address of subbits. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      let subbits: tx.validator.Subbit[] = [];
      if (opts.subbitId != undefined) {
        subbits = [
          await tx.validator.getStateBySubbitId(l, address, opts.subbitId),
        ];
      } else {
        subbits = await tx.validator.getStates(l, address);
      }
      if (opts.consumer != undefined) {
        const consumer = w[opts.consumer]
          ? w[opts.consumer].vkh
          : opts.consumer;
        subbits.filter((s) => hasConsumer(s, consumer));
      }
      if (opts.provider != undefined) {
        const provider = w[opts.provider] ? w[opts.provider].vkh : opts.vkh;
        subbits.filter((s) => hasConsumer(s, provider));
      }
      if (opts.vkh != undefined) {
        const vkh = w[opts.vkh] ? w[opts.vkh].vkh : opts.vkh;
        subbits.filter((s) => hasConsumer(s, vkh) || hasProvider(s, vkh));
      }
      subbits.forEach((s) => {
        console.log(JSON.stringify(s, null, 2));
      });
    });
}

async function mkLucid() {
  return await kio.mkLucid.mkLucidWithBlockfrost();
}

function parseOutRefs(s: string): lucid.OutRef[] {
  console.log(s);
  return s.split(",").map((ss) => {
    const [txHash, idx] = ss.split("#");
    if (idx == undefined) throw new Error("Cannot parse out ref");
    return { txHash, outputIndex: Number(idx) };
  });
}

function removeScripts(cmd: Command) {
  cmd
    .command("removeScripts")
    .description("Remove script from store. Will send ada back to uploader")
    .option("--hashes <hashes>", "The utxos by script hash (comma separated)")
    .option(
      "--outRefs <outRefs>",
      "The utxos out refs hash (comma separated) '<txid>#<idx>,...",
      parseOutRefs,
    )
    .action(async (args, ctx) => {
      const hashes: string[] = args.hashes?.split(",") || [];
      const outRefs = args.outRefs || [];
      console.log("out refs", outRefs);
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      const allUtxos = await l.utxosAt(w.store.address);
      const utxos = allUtxos.filter(
        (u) =>
          outRefs.some(
            ({ txHash, outputIndex }) =>
              u.txHash == txHash && u.outputIndex == outputIndex,
          ) ||
          (u.scriptRef &&
            hashes.includes(lucid.validatorToScriptHash(u.scriptRef!))),
      );
      utxos.forEach((u) => {
        if (u == undefined) throw new Error("Some hashes not found");
      });
      if (utxos.length == 0) throw new Error("No utxos found");
      await dapp.sequence(
        l,
        "store",
        [() => l.newTx().collectFrom(utxos)],
        `removeScripts`,
        (txb) => kio.txFinish.withChangeAddress(l, txb, w.uploader.address),
      );
    });
}

function parseDistribution(s: string): funds.Distribution {
  const dist: funds.Distribution = {};
  s.split(";").forEach((ss) => {
    const [user, adaAmt] = ss.split(",");
    if (adaAmt && BigInt(adaAmt) > 0n) dist[user] = BigInt(adaAmt);
  });
  return dist;
}

function putRef(cmd: Command) {
  cmd
    .command("put-ref")
    .description("Put ref script")
    .action(
      async (opts) =>
        await mkLucid()
          .then((l) => dapp.putRef(l))
          .then(),
    );
}

function sendFunds(cmd: Command) {
  cmd
    .command("sendFunds")
    .description("distribute ada")
    .option("--from <from>", "From whom funds are sent", "admin")
    .option(
      "--to <to>",
      "To whom funds are send and how much : `<user0>,<ada-amt>;<user1>...",
      parseDistribution,
      { consumer0: 40n, consumer1: 40n, provider0: 40n, provider1: 40n },
    )
    .action(
      async (opts) =>
        await mkLucid()
          .then((l) => funds.sendFunds(l, opts.from, opts.to))
          .then(),
    );
}
function clearFunds(cmd: Command) {
  cmd
    .command("clearFunds")
    .description("send all wallet funds from wallets to wallet")
    .option(
      "--from <from>",
      "From whom funds are sent (comma sep list)",
      (s) => s.split(","),
      ["consumer0", "consumer1", "provider0", "provider1"],
    )
    .option("--to <to>", "To whom funds are send and how much", "admin")
    .action(async (opts) => {
      await mkLucid().then((l) => funds.clearFunds(l, opts.from, opts.to));
    });
}

function parseCurrency(s: string): tx.types.Currency {
  if (s.toLowerCase() == "ada") return "Ada";
  const [kind, arg] = s.split(":");
  const kindClean = kind.toLowerCase().replace(/[^a-z0-9]/gi, "");
  if (kindClean == "byhash") return { ByHash: [arg] };
  if (kindClean == "byclass")
    return { ByClass: [arg.slice(0, 56), arg.slice(56)] };
  throw new Error(`Cannot parse: ${kind} ${arg}`);
}

function open(cmd: Command) {
  cmd
    .command("open")
    .description("open subbit")
    .requiredOption("--subbit-id <subbit-id>", "The (probably) unique id")
    .option(
      "--currency <currency>",
      "Currency of subbit. Defaults to ada",
      parseCurrency,
      "Ada",
    )
    .option("--iou-key <iou-key>", "iou key label", "iou0")
    .option("--consumer <consumer>", "consumer label", "consumer0")
    .option(
      "--provider <provider>",
      "To whom funds are send and how much",
      "provider0",
    )
    .option(
      "--close-period <close-period>",
      "Close period of subbit",
      (s) => BigInt(s),
      999n,
    )
    .option(
      "--amt <amt>",
      "Amount with which to open the subbit (in lovelace if currency is Ada)",
      (s) => BigInt(s),
      10n * ADA,
    )
    .action(async (opts) => {
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      const constants: tx.types.Constants = {
        subbitId: opts.subbitId,
        currency: opts.currency,
        iouKey: w[opts.iouKey].vkey,
        consumer: w[opts.consumer].vkh,
        provider: w[opts.provider].vkh,
        closePeriod: opts.closePeriod,
      };
      const ref = await dapp.getRef(l);
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.open.tx(l, ref, constants, opts.amt)],
        "open",
      );
    });
}

function openMany(cmd: Command) {
  cmd
    .command("open-many")
    .description("open many subbits (useful for testing)")
    .requiredOption("--many <many>", "The number of subbits", Number)
    .requiredOption("--subbit-id <subbit-id>", "Subbit id root")
    .option(
      "--currency <currency>",
      "Currency of subbit. Defaults to ada",
      parseCurrency,
      "Ada",
    )
    .option("--iou-key <iou-key>", "iou key label", "iou0")
    .option("--consumer <consumer>", "consumer label", "consumer0")
    .option(
      "--provider <provider>",
      "To whom funds are send and how much",
      "provider0",
    )
    .option(
      "--close-period <close-period>",
      "Close period of subbit",
      (s) => BigInt(s),
      999n,
    )
    .option(
      "--amt <amt>",
      "Amount with which to open the subbit (in lovelace if currency is Ada)",
      (s) => BigInt(s),
      10n * ADA,
    )
    .action(async (opts) => {
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      const constants = {
        subbitId: opts.subbitId,
        currency: opts.currency,
        iouKey: w[opts.iouKey].vkey,
        consumer: w[opts.consumer].vkh,
        provider: w[opts.provider].vkh,
        closePeriod: opts.closePeriod,
      };
      const ref = await dapp.getRef(l);
      await dapp.sequence(
        l,
        opts.consumer,
        [
          () =>
            tx.txs.open.many(
              l,
              ref,
              [...Array(opts.many).keys()].map((i) => ({
                amt: opts.amt,
                constants: {
                  ...constants,
                  subbitId: `${constants.subbitId}${i.toString(16).padStart(4, "0")}`,
                },
              })),
            ),
        ],
        "open",
      );
    });
}

function add(cmd: Command) {
  cmd
    .command("add")
    .description("Add")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--consumer <consumer>", "Consumer label")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.add.single(l, ref, subbit, opts.amt)],
        "add",
      );
    });
}

function sub(cmd: Command) {
  cmd
    .command("sub")
    .description("Sub")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--provider <provider>", "Provider label")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .requiredOption("--sig <sig>", "Signature of the iou")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.provider,
        [() => tx.txs.sub.single(l, ref, subbit, opts.amt, opts.sig)],
        "sub",
      );
    });
}

function close(cmd: Command) {
  cmd
    .command("close")
    .description("Close")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--consumer <consumer>", "Consumer label")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.close.single(l, ref, subbit)],
        "close",
      );
    });
}

function settle(cmd: Command) {
  cmd
    .command("settle")
    .description("Settle")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--provider <provider>", "Provider label")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .requiredOption("--sig <sig>", "Signature of the iou")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.provider,
        [() => tx.txs.settle.single(l, ref, subbit, opts.amt, opts.sig)],
        "settle",
      );
    });
}

function end(cmd: Command) {
  cmd
    .command("end")
    .description("End (first 50 suitable subbits)")
    .requiredOption("--consumer <consumer>", "Consumer label")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const w = dapp.wallets(l.config().network!);
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const consumer = w[opts.consumer].vkh;
      const subbits = await tx.validator
        .getSettledByConsumer(l, address, consumer)
        .then((r) => r.slice(0, 50));
      if (subbits.length == 0) throw new Error("No subbit found");
      const sss: tx.validator.SubbitStep[] = subbits.map((s) => ({
        utxo: s.utxo,
        state: { consumer },
        step: "end",
      }));
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.batch.tx(l, ref, sss)],
        "end",
      );
    });
}

function expire(cmd: Command) {
  cmd
    .command("expire")
    .description("Expire")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--consumer <consumer>", "Consumer label")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.expire.single(l, ref, subbit)],
        "expire",
      );
    });
}

type OneArg = {
  subbitId: string;
  address?: string;
  step: StepArg;
};

function parseOneArg(x: object): OneArg {
  if (!("subbitId" in x)) throw new Error("Expect key subbitId");
  if (typeof x.subbitId != "string")
    throw new Error("Expect subbitId of type string");
  if (!("step" in x)) throw new Error("Expect key step");
  if (typeof x.step != "object") throw new Error("Expect step of type object");
  if (x.step == null) throw new Error("Expect step to not be null");
  return {
    subbitId: x.subbitId,
    step: parseStepArg(x.step),
  };
}

type StepArg =
  | { step: "add"; amt: bigint }
  | { step: "sub"; amt: bigint; sig: string }
  | { step: "close" }
  | { step: "settle"; amt: bigint; sig: string }
  | { step: "end" }
  | { step: "expire" };

function parseStepArg(x: object): StepArg {
  if (!("step" in x)) throw new Error("Expect key step");
  if (typeof x.step != "string") throw new Error("Expect step of type string");
  if (x.step == "add") {
    if (!("amt" in x)) throw new Error("Expect key amt");
    if (typeof x.amt != "string") throw new Error("Expect amt of type string");
    return { step: "add", amt: BigInt(x.amt) };
  } else if (x.step == "sub") {
    if (!("amt" in x)) throw new Error("Expect key amt");
    if (typeof x.amt != "string") throw new Error("Expect amt of type string");
    if (!("sig" in x)) throw new Error("Expect key sig");
    if (typeof x.sig != "string") throw new Error("Expect sig of type string");
    return { step: "sub", amt: BigInt(x.amt), sig: x.sig };
  } else if (x.step == "close") {
    return { step: "close" };
  } else if (x.step == "settle") {
    if (!("amt" in x)) throw new Error("Expect key amt");
    if (typeof x.amt != "string") throw new Error("Expect amt of type string");
    if (!("sig" in x)) throw new Error("Expect key sig");
    if (typeof x.sig != "string") throw new Error("Expect sig of type string");
    return { step: "settle", amt: BigInt(x.amt), sig: x.sig };
  } else if (x.step == "end") {
    return { step: "end" };
  } else if (x.step == "expire") {
    return { step: "expire" };
  }
  throw new Error("Cannot parse step");
}

function parseBatchArg(arg: string): OneArg[] {
  const x = JSON.parse(arg);
  if (typeof x != "object") throw new Error("Expect object");
  return x.map(parseOneArg);
}

function batch(cmd: Command) {
  cmd
    .command("batch")
    .description("Batch tx")
    .argument("<user>", "Wallet label of user")
    .argument("<arg>", "Stringified JSON. See help for form.", parseBatchArg)
    .addHelpText(
      "after",
      [
        "",
        "ARG :: The arg is a stringified JSON.",
        "The content is an array of objects.",
        "Each object contains the `subbitId`, `step`, and optional `address`.",
        "The step value is an object with key `step` that must be one of:",
        "   add, sub, close, settle, end, expire.",
        "Additional keys are required in the case of add, sub, and settle.",
        "For add, the `amt` must be included.",
        "For sub and settle, the `amt` and `sig` must be included.",
        "For example:",
        "   [",
        "       { 'subbitId' : 'deadbeef00' , 'step' : { 'step' : 'add', 'amt' : '10000000' } }, ",
        "       { 'subbitId' : 'deadbeef01' , 'step' : { 'step' : 'sub', 'amt' : '10000000' , sig : '0000000000000000000000000000...'} }, ",
        "       { 'subbitId' : 'deadbeef02' , 'step' : { 'step' : 'close' } }",
        "   ]",
        "",
      ].join("\n"),
    )
    .action(async (user, arg) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const defaultAddress = tx.validator.mkAddress(l.config().network!);
      const addresses = [
        ...new Set(arg.map((s) => s.address || defaultAddress)),
      ] as string[];
      const cacheStates: Record<string, tx.validator.Subbit[]> =
        await Promise.all(
          addresses.map((a: string) =>
            tx.validator.getStates(l, a).then((s) => [a, s]),
          ),
        ).then(Object.fromEntries);
      const sss: tx.validator.SubbitStep[] = await Promise.all(
        arg.map(async (oneArg: OneArg) => {
          const address = oneArg.address || defaultAddress;
          const subbit = cacheStates[address].find(
            (s) =>
              s.state.kind != "Settled" &&
              s.state.value.constants.subbitId == oneArg.subbitId,
          );
          if (subbit == undefined)
            throw new Error(`Cannot find ${oneArg.subbitId}`);
          return {
            utxo: subbit.utxo,
            state: subbit.state.value,
            ...oneArg.step,
          };
        }),
      );
      await dapp.sequence(
        l,
        user,
        [() => tx.txs.batch.tx(l, ref, sss)],
        "batch",
        //(txb) => kio.txFinish.costBreakdown(txb),
      );
    });
}

function mutualEnd(cmd: Command) {
  cmd
    .command("mutual-end")
    .description("End subbit with mutual consent. Funds return to consumer")
    .requiredOption("--subbit-id <subbit-id>", "The (probably) unique id")
    .requiredOption("--consumer <consumer>", "Consumer label")
    .requiredOption("--provider <provider>", "Provider label")
    .option("--address <address>", "Address of subbit. Default to stakeless")
    .action(async (opts) => {
      const l = await mkLucid();
      const ref = await dapp.getRef(l);
      const address =
        opts.address || tx.validator.mkAddress(l.config().network!);
      const subbit = await tx.validator.getStateBySubbitId(
        l,
        address,
        opts.subbitId,
      );
      await dapp.sequence(
        l,
        opts.consumer,
        [() => tx.txs.mutual.tx(l, ref, subbit)],
        "mutual",
      );
    });
}

function addIous(cmd: Command) {
  sign(cmd);
  signMany(cmd);
  verify(cmd);
}

function sign(cmd: Command) {
  cmd
    .command("sign")
    .description("Output a signature for the iou")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .requiredOption("--iou-key <iou-key>", "Iou key label")
    .action((opts) =>
      console.log(dapp.sign(opts.iouKey, opts.subbitId, opts.amt)),
    );
}

function signMany(cmd: Command) {
  cmd
    .command("sign-many")
    .description("Output a signature for the ious (useful in testing)")
    .requiredOption("--many <many>", "The number of subbits", Number)
    .requiredOption("--subbit-id <subbit-id>", "Subbit id root")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .requiredOption("--iou-key <iou-key>", "Iou key label")
    .action((opts) =>
      console.log(
        JSON.stringify(
          [...Array(opts.many).keys()].map((i): OneArg => {
            const subbitId = `${opts.subbitId}${i.toString(16).padStart(4, "0")}`;
            return {
              subbitId,
              step: {
                step: "sub",
                amt: opts.amt,
                sig: dapp.sign(opts.iouKey, subbitId, opts.amt),
              },
            };
          }),
        ),
      ),
    );
}

function verify(cmd: Command) {
  cmd
    .command("verify")
    .description("Verify an iou")
    .requiredOption("--subbit-id <subbit-id>", "Subbit id")
    .requiredOption("--amt <amt>", "Iou amount", (s) => BigInt(s))
    .requiredOption("--iou-key <iou-key>", "Iou label or vkey")
    .requiredOption("--sig <sig>", "Signature")
    .action((opts) => {
      console.log(dapp.verify(opts.iouKey, opts.subbitId, opts.amt, opts.sig));
    });
}

main();
