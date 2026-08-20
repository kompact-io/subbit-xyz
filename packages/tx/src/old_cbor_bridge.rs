use minicbor::data::Token;
use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};

pub fn encode<T, W, C>(
    val: &T,
    e: &mut Encoder<W>,
    _ctx: &mut C,
) -> Result<(), encode::Error<W::Error>>
where
    T: minicbor_old::Encode<()>,
    W: Write,
{
    let raw = minicbor_old::to_vec(val).map_err(|err| encode::Error::message(err.to_string()))?;

    let mut d = Decoder::new(&raw);
    while (d.position() as usize) < raw.len() {
        let tok: Token = d
            .decode()
            .map_err(|err| encode::Error::message(err.to_string()))?;
        e.encode(tok)?;
    }
    Ok(())
}

pub fn decode<'b, T, C>(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<T, decode::Error>
where
    T: for<'a> minicbor_old::Decode<'a, ()>,
{
    let start = d.position();
    d.skip()?;
    let end = d.position();
    let raw = &d.input()[start..end];

    minicbor_old::decode(raw).map_err(|err| decode::Error::message(err.to_string()))
}
