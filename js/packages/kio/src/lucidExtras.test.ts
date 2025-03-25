import { sumUtxos } from "./lucidExtras";

test("description", () => {
  const a = {
    txHash: "",
    outputIndex: 0,
    address: "",
    assets: Object.fromEntries([
      ["a".repeat(56), 100n],
      ["b".repeat(56), 200n],
    ]),
  };
  expect(sumUtxos([a, a, a, a])).toStrictEqual(
    Object.fromEntries([
      ["lovelace", 0n],
      ["a".repeat(56), 400n],
      ["b".repeat(56), 800n],
    ]),
  );
});
