import { describe, expect, test } from "@jest/globals";
import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";

import * as iou from "./iou";
import * as t from "./types";

const iou0 = {
  skey: "0000000000000000000000000000000000000000000000000000000000000000",
  vkey: "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
  subbit_id: "6c3b9aa767f785b5",
  amount: 8594738769458413623n,
  msg: "d8799f486c3b9aa767f785b51b7746a55fbad8c037ff",
  sig: "8bb7cb5fc82b23fac9114e9a4913f3b01c4db608e6189f01eeb431c7cd0c8c6e28484cdcc25aa895a715622598110754ede40f390797179a70e373856450ce03",
};

describe("iou", () => {
  test("iou0", () => {
    const skeyBech32 = cml.PrivateKey.from_normal_bytes(
      lucid.fromHex(iou0.skey),
    ).to_bech32();
    const r: t.IouMessage = {
      subbitId: iou0.subbit_id,
      amount: iou0.amount,
    };
    const sig = iou.sign(skeyBech32, r);
    expect(sig).toBe(iou0.sig);
  });
});
