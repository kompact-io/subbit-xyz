pub fn expect_empty(len: Option<u64>) -> Result<(), minicbor::decode::Error> {
    if len != Some(0) {
        return Err(minicbor::decode::Error::message(
            "expected definite empty array(0) for this variant",
        ));
    }
    Ok(())
}

pub fn expect_end<'b>(d: &mut minicbor::Decoder<'b>) -> Result<(), minicbor::decode::Error> {
    if d.datatype()? != minicbor::data::Type::Break {
        return Err(minicbor::decode::Error::message(
            "unexpected trailing field(s); expected end of array (0xff)",
        ));
    }
    d.skip()?; // consume the Break marker itself
    Ok(())
}
