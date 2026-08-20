#[derive(Debug)]
pub enum ParseError {
    Hex(hex::FromHexError),
    Int(core::num::ParseIntError),
    Length { expected: usize, got: usize },
    Constraint(&'static str),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Hex(e) => write!(f, "invalid hex: {e:?}"),
            ParseError::Int(e) => write!(f, "invalid integer: {e}"),
            ParseError::Length { expected, got } => {
                write!(f, "expected {expected} bytes, got {got}")
            }
            ParseError::Constraint(msg) => f.write_str(msg),
        }
    }
}

impl core::error::Error for ParseError {}

impl From<hex::FromHexError> for ParseError {
    fn from(e: hex::FromHexError) -> Self {
        ParseError::Hex(e)
    }
}

impl From<core::num::ParseIntError> for ParseError {
    fn from(e: core::num::ParseIntError) -> Self {
        ParseError::Int(e)
    }
}
