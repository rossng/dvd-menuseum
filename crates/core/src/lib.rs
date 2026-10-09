//! DVD-Video navigation core for the DVD Menuseum.
//!
//! Pure logic, no I/O: the host supplies IFO bytes and NAV packs and drives the
//! navigation engine (`menuseum-engine`, through [`navigator::Navigator`]) one
//! event at a time.

mod bytes;
pub mod cmd;
pub mod ifo;
pub mod nav;
pub mod navigator;
pub mod vm;

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated { what: &'static str, offset: usize, len: usize },
    BadMagic(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Truncated { what, offset, len } => {
                write!(f, "{what}: read of {len} bytes at offset {offset} runs past the end")
            }
            Error::BadMagic(file) => write!(f, "{file}: bad identifier"),
        }
    }
}

impl std::error::Error for Error {}
