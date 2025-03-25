import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";

export function genKeyBech32() {
  return cml.PrivateKey.generate_ed25519().to_bech32();
}

console.log(genKeyBech32());
