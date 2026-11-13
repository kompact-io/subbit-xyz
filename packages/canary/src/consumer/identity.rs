// TODO. Plumb in.
/// This seat's on-chain identity: fixed for a given `(seed, tag)` and, unlike `Settings`'s
/// other fields, stable across many separate runs/resumes of the same seat -- derived
/// from `seed` alone, never sampled.
pub struct Identity {
    pub tag: Tag,
    /// `Constants.consumer`'s own verification-key hash for this seat's `seed`.
    pub vkh: Hash28,
    pub iou_key: VerifyingKey,
}

impl Identity {
    fn derive(seed: [u8; 32], tag: Tag) -> Self {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        Self {
            vkh: hash28_from_seed(&seed),
            iou_key: VerifyingKey::from(signing_key.verifying_key().to_bytes()),
            tag,
        }
    }

    /// This seat's own hex keytag (`iou_key` + `tag`), matching the shape
    /// `subbit_console::admin::Ctx::settle`/`sub`/`claim` resolve against.
    pub fn keytag(&self) -> String {
        self.iou_key.to_string() + &self.tag.to_string()
    }
}
