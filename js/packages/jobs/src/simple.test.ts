import { describe, expect, test } from "@jest/globals";
import * as kio from "@subbit-tx/kio";
import { setup } from "./setup";
import { job } from "./simple";

describe("simple", () => {
  test("simple", async () => {
    let l = await kio.mkLucid.mkLucidWithEmulator();
    let _hash = await setup(l);
    let _res = await job(l);
  });
});
