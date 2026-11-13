use wasm_bindgen::prelude::*;

use crate::{Id, Stamp, dialect::vanilla};

#[wasm_bindgen]
pub struct JsStamp(Stamp);

#[wasm_bindgen]
impl JsStamp {
    /// `signing_key`: 32 raw ed25519 seed bytes. `tag`: opaque tag bytes.
    /// `ttl_relative_secs`: Pop TTL in seconds.
    #[wasm_bindgen(constructor)]
    pub fn new(signing_key: &[u8], tag: &[u8], ttl_relative_secs: u64) -> Result<JsStamp, JsError> {
        let key: [u8; 32] = signing_key
            .try_into()
            .map_err(|_| JsError::new("signing_key must be 32 bytes"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&key);
        let tag = subbit_core::Tag::from(tag.to_vec());
        let id = Id::new(key, tag);
        let ttl = subbit_core::Duration::from_secs(ttl_relative_secs);
        Ok(JsStamp(Stamp::new(id, ttl)))
    }

    /// Base64 request envelope for spending `cost`. Send it yourself
    /// (fetch/XHR), then pass the reply to `response`.
    #[wasm_bindgen(js_name = spend)]
    pub fn spend(&mut self, cost: u64) -> String {
        vanilla::request(&mut self.0, cost)
    }

    /// Apply a base64 response envelope. Throws on decode/verify failure.
    #[wasm_bindgen(js_name = response)]
    pub fn response(&mut self, envelope: &str) -> Result<(), JsError> {
        vanilla::response(&mut self.0, envelope).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = balance)]
    pub fn committed(&self) -> u64 {
        self.0.committed().unwrap_or(0)
    }

    /// Cache a catalog learned some other way (e.g. a `.well-known/subbit`
    /// fetch). `catalog_json` is a JSON-encoded `Catalog`.
    #[wasm_bindgen(js_name = setCatalog)]
    pub fn set_catalog(&mut self, catalog_json: &str) -> Result<(), JsError> {
        let catalog: subbit_l2_dto::Catalog =
            serde_json::from_str(catalog_json).map_err(|e| JsError::new(&e.to_string()))?;
        self.0
            .set_catalog(catalog)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// The cached catalog, JSON-encoded for the JS side to `JSON.parse`
    /// and present for consent. `None` if nothing's been cached yet.
    #[wasm_bindgen(js_name = catalog)]
    pub fn catalog(&self) -> Option<String> {
        self.0
            .catalog()
            .map(|c| serde_json::to_string(c).expect("Catalog serialization is infallible"))
    }
}
