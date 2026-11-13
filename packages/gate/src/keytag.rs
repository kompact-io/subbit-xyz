use core::cmp::Ordering;
use core::fmt;
use core::ops::Deref;
use core::str::FromStr;
use redb::{Key as RedbKey, TypeName, Value as RedbValue};
use serde::{Deserialize, Serialize};

use subbit_core::{Tag, VerifyingKey};

/// 32-byte key id + arbitrary tail, flattened into one buffer.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    minicbor::Encode,
    minicbor::Decode,
)]
#[cbor(transparent)]
#[serde(transparent)]
pub struct Keytag(
    #[cbor(with = "minicbor::bytes")]
    #[serde(with = "crate::hex_bytes")]
    Vec<u8>,
);

impl Keytag {
    /// Build a `Keytag` by concatenating a `VerifyingKey` with `Tag`.
    pub fn new(key: VerifyingKey, tag: Tag) -> Self {
        let key: [u8; 32] = key.into();
        let mut buf = Vec::with_capacity(32 + tag.len());
        buf.extend_from_slice(&key);
        buf.extend(tag.as_ref());
        Keytag(buf)
    }

    /// Inverse of `new`: split back into the `VerifyingKey` and `Tag`
    pub fn split(self) -> (VerifyingKey, Tag) {
        let mut buf = self.0;
        assert!(buf.len() >= 32, "Keytag shorter than 32 bytes");
        let tail = buf.split_off(32);
        let mut key = [0u8; 32];
        key.copy_from_slice(&buf);
        (VerifyingKey::from(key), Tag::from(tail))
    }
}

impl fmt::Display for Keytag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

/// Error returned by `Keytag::from_str` for malformed hex input.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum KeytagParseError {
    #[error("hex: {0}")]
    Hex(String),
}

impl From<hex::FromHexError> for KeytagParseError {
    fn from(value: hex::FromHexError) -> Self {
        Self::Hex(value.to_string())
    }
}

impl FromStr for Keytag {
    type Err = KeytagParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Keytag(hex::decode(s)?))
    }
}

impl Deref for Keytag {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Keytag {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Keytag {
    fn from(v: Vec<u8>) -> Self {
        Keytag(v)
    }
}

impl From<Keytag> for Vec<u8> {
    fn from(k: Keytag) -> Self {
        k.0
    }
}

// ---------------------------------------------------------------------------
// redb
// ---------------------------------------------------------------------------

impl RedbValue for Keytag {
    type SelfType<'a> = Keytag;
    type AsBytes<'a> = &'a [u8];

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        Keytag(data.to_vec())
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.as_ref()
    }

    fn type_name() -> TypeName {
        TypeName::new("Keytag")
    }
}

// Byte comparison here matches the derived `Ord` above: `Vec<u8>`'s
// lexicographic ordering is the same as comparing the raw byte slices,
// so redb's on-disk sort order agrees with `Keytag`'s own `Ord`.
impl RedbKey for Keytag {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = VerifyingKey::from([7u8; 32]);
        let tag = Tag::from(vec![1, 2, 3]);
        let keytag = Keytag::new(key, tag.clone());
        assert_eq!(keytag.len(), 35);

        let (key2, tag2) = keytag.split();
        assert_eq!(key2, key);
        assert_eq!(tag2, tag);
    }

    #[test]
    fn display_fromstr_roundtrip() {
        let key = VerifyingKey::from([7u8; 32]);
        let tag = Tag::from(vec![1, 2, 3]);
        let keytag = Keytag::new(key, tag.clone());

        let s = keytag.to_string();
        let parsed: Keytag = s.parse().unwrap();
        assert_eq!(parsed, keytag);
    }
}
