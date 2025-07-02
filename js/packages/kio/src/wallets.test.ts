import { privateKeys, walletInfo } from "./wallets";

test("id", () => {
  const info = walletInfo("Preprod", "admin", privateKeys["admin"]!);
  expect(info.id).toBe("admin");
});

test("privateKey", () => {
  const w0 = walletInfo("Preprod", "consumer0", privateKeys["consumer0"]!);
  expect(w0.vkey).toBe(
    "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
  );
  const w1 = walletInfo("Preprod", "consumer1", privateKeys["consumer1"]!);
  expect(w1.vkey).toBe(
    "4cb5abf6ad79fbf5abbccafcc269d85cd2651ed4b885b5869f241aedf0a5ba29",
  );
});
