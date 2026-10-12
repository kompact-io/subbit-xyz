use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use subbit_core::Hash32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OutputRef {
    #[n(0)]
    pub tx_hash: Hash32,
    #[n(1)]
    pub index: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let output_ref = OutputRef {
            tx_hash: Hash32::from([7u8; 32]),
            index: 2,
        };
        let bytes = minicbor::to_vec(output_ref).expect("encode failed");
        let decoded: OutputRef = minicbor::decode(&bytes).expect("decode failed");
        assert_eq!(output_ref, decoded);
    }
}
