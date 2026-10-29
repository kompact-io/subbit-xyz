use core::fmt;
use core::marker::PhantomData;
use serde::{Deserializer, Serializer, de};

pub fn serialize<T, S>(bytes: &T, s: S) -> Result<S::Ok, S::Error>
where
    T: AsRef<[u8]>,
    S: Serializer,
{
    if s.is_human_readable() {
        s.serialize_str(&hex::encode(bytes.as_ref()))
    } else {
        s.serialize_bytes(bytes.as_ref())
    }
}

pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<Vec<u8>>,
{
    struct BytesVisitor<T>(PhantomData<T>);

    impl<'de, T: TryFrom<Vec<u8>>> de::Visitor<'de> for BytesVisitor<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a hex string or raw bytes")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let bytes = hex::decode(v).map_err(E::custom)?;
            let len = bytes.len();
            T::try_from(bytes).map_err(|_| E::custom(format!("invalid byte length: {len}")))
        }

        fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let len = v.len();
            T::try_from(v.to_vec()).map_err(|_| E::custom(format!("invalid byte length: {len}")))
        }

        fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let len = v.len();
            T::try_from(v).map_err(|_| E::custom(format!("invalid byte length: {len}")))
        }
    }

    if d.is_human_readable() {
        d.deserialize_str(BytesVisitor(PhantomData))
    } else {
        d.deserialize_bytes(BytesVisitor(PhantomData))
    }
}
