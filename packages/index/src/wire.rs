use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Encode, Decode,
)]
#[cbor(transparent)]
#[serde(transparent)]
pub struct Keytag(
    #[n(0)]
    #[cbor(with = "minicbor::bytes")]
    #[serde(with = "hex::serde")]
    pub Vec<u8>,
);

impl FromStr for Keytag {
    type Err = hex::FromHexError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s).map(Keytag)
    }
}

impl fmt::Display for Keytag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct Backing {
    #[n(0)]
    pub amount: u64,
    #[n(1)]
    pub subbed: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    pub keytag: Keytag,
    pub amount: Option<u64>,
    pub subbed: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: amount/subbed must both be set or both blank")]
    Mismatched(Keytag),
    #[error("empty row")]
    EmptyRow,
    #[error(transparent)]
    Csv(#[from] csv::Error),
}

impl Row {
    /// Combine amount/subbed into a Backing. Both blank -> Ok(None).
    /// Only one set -> Err (ambiguous, not a valid row).
    pub fn to_backing(&self) -> Result<Option<Backing>, Error> {
        match (self.amount, self.subbed) {
            (None, None) => Ok(None),
            (Some(amount), Some(subbed)) => Ok(Some(Backing { amount, subbed })),
            _ => Err(Error::Mismatched(self.keytag.clone())),
        }
    }
}

pub fn parse_row(row: &str) -> Result<Row, Error> {
    parse_rows(row.as_bytes(), false)?
        .into_iter()
        .next()
        .ok_or(Error::EmptyRow)
}

pub fn parse_rows(bytes: &[u8], header: bool) -> Result<Vec<Row>, Error> {
    csv::ReaderBuilder::new()
        .has_headers(header)
        .from_reader(bytes)
        .deserialize()
        .map(|r| r.map_err(Into::into))
        .collect()
}

pub fn rows_to_map(entries: Vec<Row>) -> Result<BTreeMap<Keytag, Option<Backing>>, Error> {
    entries
        .into_iter()
        .map(|e| Ok((e.keytag.clone(), e.to_backing()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_row() {
        let e = parse_row("deadbeef,1000,50").unwrap();
        assert_eq!(
            e.to_backing().unwrap(),
            Some(Backing {
                amount: 1000,
                subbed: 50
            })
        );
    }

    #[test]
    fn blank_row() {
        let e = parse_row("aabbccdd,,").unwrap();
        assert_eq!(e.to_backing().unwrap(), None);
    }

    #[test]
    fn mismatched_row_errors() {
        let e = parse_row("deadbeef,1000,").unwrap();
        assert!(e.to_backing().is_err());
    }

    #[test]
    fn file_with_header() {
        let data = b"keytag,amount,subbed\ndeadbeef,1000,50\naabbccdd,,\n";
        let entries = parse_rows(data, true).unwrap();
        let map = rows_to_map(entries).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map[&Keytag::from_str("deadbeef").unwrap()],
            Some(Backing {
                amount: 1000,
                subbed: 50
            })
        );
        assert_eq!(map[&Keytag::from_str("aabbccdd").unwrap()], None);
    }

    #[test]
    fn mismatched_in_map_bubbles_error() {
        let entries = vec![parse_row("deadbeef,1000,").unwrap()];
        assert!(rows_to_map(entries).is_err());
    }
}
