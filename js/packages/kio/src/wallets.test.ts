import { privateKeys, walletInfo } from "./wallets";

test("description", () => {
  const info = walletInfo("Preprod", "admin", privateKeys["admin"]!);
  expect(info.id).toBe("admin");
});
