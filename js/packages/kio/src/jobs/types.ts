import * as lucid from "@lucid-evolution/lucid";

export type JobCtx = {
  lucid: lucid.LucidEvolution;
  submitter(tx: lucid.TxSigned, label: string | null): void | Promise<void>;
};
