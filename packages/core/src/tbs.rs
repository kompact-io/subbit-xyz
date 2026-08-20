use minicbor::Encode;

// General To-Be-Signed.
#[derive(Debug, Clone, PartialEq, Encode)]
pub struct Tbs<B> {
    #[n(0)]
    dts: Vec<u8>,
    #[n(1)]
    body: B,
}

const DTS: &[u8] = b"SUBBIT_AUTH";

pub fn subbit(body: impl Encode<()>) -> Vec<u8> {
    minicbor::to_vec(&Tbs {
        dts: DTS.into(),
        body,
    })
    .unwrap()
}
