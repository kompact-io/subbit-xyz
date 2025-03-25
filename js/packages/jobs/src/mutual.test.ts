import { describe, expect, test } from "@jest/globals";
import * as lucid from "@lucid-evolution/lucid";
import * as kio from "@subbit-tx/kio";
import { setup } from "./setup";
import * as mutual from "./mutual";

describe("simple", () => {
  let l: lucid.LucidEvolution;
  let ref: lucid.UTxO;
  beforeAll(async () => {
    l = await kio.mkLucid.mkLucidWithEmulator();
    ref = await setup(l);
  });
  beforeEach(async () => {
    await mutual.setup(l, ref);
  });
  test("job", async () => {
    await mutual.job(l, ref);
  });
  test("oneSign", async () => {
    await expect(mutual.oneSign(l, ref)).rejects.toThrow();
  });
});
