#[cfg(not(feature = "std"))]
mod no_std {
    extern crate alloc;
    pub use alloc::vec::Vec;
}

#[cfg(not(feature = "std"))]
pub use no_std::*;

#[cfg(feature = "std")]
pub use std::vec::Vec;
