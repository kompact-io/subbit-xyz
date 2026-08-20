use cryptoxide::ed25519;

use subbit_core::{Signature, VerifyingKey};

pub fn verify(key: &VerifyingKey, msg: &[u8], sig: &Signature) -> bool {
    ed25519::verify(msg, key.as_ref(), sig.as_ref())
}
