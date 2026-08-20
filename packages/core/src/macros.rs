/// Fixed-length byte newtype: struct + Encode/Decode/Arbitrary/serde derives,
/// plus the standard conversions (new/to_bytes/Display/FromStr/AsRef/From/TryFrom).
macro_rules! newtype_array {
    ($(#[$doc:meta])* $name:ident, $len:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::minicbor::Encode, ::minicbor::Decode)]
        #[cfg_attr(feature = "test-utils", derive(::proptest_derive::Arbitrary))]
        #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
        #[cbor(transparent)]
        pub struct $name(
            #[cbor(with = "crate::chunked_bytes")]
            #[cfg_attr(feature = "serde", serde(with = "crate::hex_bytes"))]
            [u8; $len],
        );

        impl $name {
            pub fn new(bytes: [u8; $len]) -> Self {
                Self(bytes)
            }

            pub fn to_bytes(self) -> [u8; $len] {
                self.0
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&::hex::encode(self.0))
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = ::hex::FromHexError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let bytes = ::hex::decode(s)?;
                bytes
                    .try_into()
                    .map_err(|_| ::hex::FromHexError::InvalidStringLength)
            }
        }

        // NOTE: both AsRef<[u8; $len]> and AsRef<[u8]> are provided. This is a
        // known sharp edge: in a generic context bounded by `T: AsRef<[u8]>`,
        // calling `x.as_ref()` directly on `$name` can be ambiguous and require
        // an explicit turbofish/annotation to pick one. Don't "simplify" this
        // to a single impl without checking call sites.
        impl AsRef<[u8; $len]> for $name {
            fn as_ref(&self) -> &[u8; $len] {
                &self.0
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl From<[u8; $len]> for $name {
            fn from(b: [u8; $len]) -> Self {
                Self(b)
            }
        }

        impl From<$name> for [u8; $len] {
            fn from(v: $name) -> Self {
                v.0
            }
        }

        impl TryFrom<&[u8]> for $name {
            type Error = ::core::array::TryFromSliceError;
            fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
                <[u8; $len]>::try_from(value).map(Self)
            }
        }

        impl TryFrom<Vec<u8>> for $name {
            type Error = Vec<u8>;
            fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
                <[u8; $len]>::try_from(value).map(Self)
            }
        }
    };
}
pub(crate) use newtype_array;

/// Variable-length byte newtype: struct + derives, plus the standard conversions.
macro_rules! newtype_bytes {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ::minicbor::Encode, ::minicbor::Decode)]
        #[cfg_attr(feature = "test-utils", derive(::proptest_derive::Arbitrary))]
        #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
        #[cbor(transparent)]
        pub struct $name(
            #[cbor(with = "crate::chunked_bytes")]
            #[cfg_attr(feature = "serde", serde(with = "crate::hex_bytes"))]
            Vec<u8>,
        );

        impl $name {
            pub fn len(&self) -> usize {
                self.0.len()
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&::hex::encode(&self.0))
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = ::hex::FromHexError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                ::hex::decode(s).map(Self)
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl From<Vec<u8>> for $name {
            fn from(v: Vec<u8>) -> Self {
                Self(v)
            }
        }

        impl From<$name> for Vec<u8> {
            fn from(v: $name) -> Self {
                v.0
            }
        }

        impl<'a> From<&'a [u8]> for $name {
            fn from(s: &'a [u8]) -> Self {
                Self(s.to_vec())
            }
        }
    };
}
pub(crate) use newtype_bytes;
