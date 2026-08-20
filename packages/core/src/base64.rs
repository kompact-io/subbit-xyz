use base64::Engine;
use minicbor::{Decode, Encode};

pub fn to_base64(x: &impl Encode<()>) -> String {
    let bytes = minicbor::to_vec(x).expect("Ecoding failure");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("Other")]
    Other,
    #[error("Base 64")]
    Base64,
    #[error("Cbor")]
    Cbor,
}

pub fn from_base64<T: for<'a> Decode<'a, ()>>(s: &str) -> Result<T, Error> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| Error::Base64)?;
    let x = minicbor::decode::<T>(&bytes).map_err(|_| Error::Cbor)?;
    Ok(x)
}
