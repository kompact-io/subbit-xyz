use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};

const CHUNK_SIZE: usize = 64;

pub fn encode<T, W, C>(
    bytes: &T,
    e: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), encode::Error<W::Error>>
where
    T: AsRef<[u8]>,
    W: Write,
{
    let bytes = bytes.as_ref();
    if bytes.len() <= CHUNK_SIZE {
        e.bytes(bytes)?;
    } else {
        e.begin_bytes()?;
        for chunk in bytes.chunks(CHUNK_SIZE) {
            e.bytes(chunk)?;
        }
        e.end()?;
    }
    Ok(())
}

pub fn decode<'b, T, C>(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<T, decode::Error>
where
    T: TryFrom<Vec<u8>>,
{
    let mut out = Vec::new();
    for chunk in d.bytes_iter()? {
        out.extend_from_slice(chunk?);
    }
    let len = out.len();
    T::try_from(out).map_err(|_| decode::Error::message(format!("invalid byte length: {len}")))
}
