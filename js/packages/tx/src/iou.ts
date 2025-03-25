import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";
import * as t from "./types";

export function iouSer(x: t.IouMessage): string {
  return lucid.Data.to<t.IouMessage>(x, t.IouMessage);
}

export function iouDe(x: string): t.IouMessage {
  return lucid.Data.from<t.IouMessage>(x, t.IouMessage);
}

export function sign(skeyBech32: string, r: t.IouMessage) {
  const d = iouSer(r);
  const b = lucid.fromHex(d);
  const skey = cml.PrivateKey.from_bech32(skeyBech32);
  const sig = skey.sign(b).to_hex();
  return sig;
}

export function verify(vkeyHex: string, r: t.IouMessage, sig: string) {
  return cml.PublicKey.from_bytes(lucid.fromHex(vkeyHex)).verify(
    lucid.fromHex(iouSer(r)),
    cml.Ed25519Signature.from_hex(sig),
  );
}
