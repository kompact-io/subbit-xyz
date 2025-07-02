import * as lucid from "@lucid-evolution/lucid";
import * as cml from "@anastasia-labs/cardano-multiplatform-lib-nodejs";
import * as t from "./types";

function privateKey(s: string) {
  if (s.startsWith("ed25519_sk1")) {
    return cml.PrivateKey.from_bech32(s);
  } else if (s.length == 64) {
    return cml.PrivateKey.from_normal_bytes(Buffer.from(s, "hex"));
  }
  throw new Error("Cannot coerce string to private key");
}

export function iouSer(x: t.IouMessage): string {
  return lucid.Data.to<t.IouMessage>(x, t.IouMessage);
}

export function iouDe(x: string): t.IouMessage {
  return lucid.Data.from<t.IouMessage>(x, t.IouMessage);
}

export function sign(skeyStr: string, r: t.IouMessage) {
  const d = iouSer(r);
  const b = lucid.fromHex(d);
  const skey = privateKey(skeyStr);
  const sig = skey.sign(b).to_hex();
  return sig;
}

export function verify(vkeyHex: string, r: t.IouMessage, sig: string) {
  return cml.PublicKey.from_bytes(lucid.fromHex(vkeyHex)).verify(
    lucid.fromHex(iouSer(r)),
    cml.Ed25519Signature.from_hex(sig),
  );
}
