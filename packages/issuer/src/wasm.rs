use wasm_bindgen::prelude::*;

use crate::{Account, Issuer};

#[wasm_bindgen]
pub struct JsIssuer(Issuer);

#[wasm_bindgen]
impl JsIssuer {
    /// `signing_key`: 32 raw ed25519 seed bytes. `tag`: opaque tag bytes.
    /// `ttl_relative_secs`: Pop TTL in seconds.
    #[wasm_bindgen(constructor)]
    pub fn new(
        signing_key: &[u8],
        tag: &[u8],
        ttl_relative_secs: u64,
    ) -> Result<JsIssuer, JsError> {
        let key: [u8; 32] = signing_key
            .try_into()
            .map_err(|_| JsError::new("signing_key must be 32 bytes"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&key);
        let tag = subbit_core::Tag::from(tag.to_vec());
        let account = Account::new(key, tag);
        let ttl = subbit_core::Duration::from_secs(ttl_relative_secs);
        Ok(JsIssuer(Issuer::new(account, ttl)))
    }

    /// Base64 request envelope for spending `cost`. Send it yourself
    /// (fetch/XHR), then pass the reply to `response`.
    #[wasm_bindgen(js_name = spend)]
    pub fn spend(&mut self, cost: u64) -> String {
        self.0.spend(cost)
    }

    /// Apply a base64 response envelope. Throws on decode/verify failure.
    #[wasm_bindgen(js_name = response)]
    pub fn response(&mut self, envelope: &str) -> Result<(), JsError> {
        self.0
            .response(envelope)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = balance)]
    pub fn committed(&self) -> u64 {
        self.0.committed().unwrap_or(0)
    }
}
