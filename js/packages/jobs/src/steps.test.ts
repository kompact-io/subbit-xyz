import { describe, expect, test } from "@jest/globals";
import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import { setup } from "./setup";
import * as steps from "./steps";

describe("simple", () => {
  let l: lucid.LucidEvolution;
  let ref: lucid.UTxO;
  beforeEach(async () => {
    l = await kio.mkLucid.mkLucidWithEmulator();
    ref = await setup(l);
  });
  test("add", async () => await steps.add(l, ref));
  test("sub", async () => await steps.sub(l, ref));
  test("close", async () => await steps.close(l, ref));
  test("settle", async () => await steps.settle(l, ref));
  test("end", async () => await steps.end(l, ref));
  test("expire", async () => await steps.expire(l, ref));
  test("addFail", async () =>
    await expect(steps.addFail(l, ref)).rejects.toThrow());
  test("subFail", async () =>
    await expect(steps.subFail(l, ref)).rejects.toThrow());
  test("closeFail", async () =>
    await expect(steps.closeFail(l, ref)).rejects.toThrow());
  test("settleFail", async () =>
    await expect(steps.settleFail(l, ref)).rejects.toThrow());
  test("endFail", async () =>
    await expect(steps.endFail(l, ref)).rejects.toThrow());
  test("expireFail", async () =>
    await expect(steps.expireFail(l, ref)).rejects.toThrow());
});
