use crate::Signed;

pub type Signature = [u8; 20];

pub type Mac<B> = Signed<B, Signature>;
