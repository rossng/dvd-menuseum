//! Bounds-checked big-endian reads over a byte slice.

use crate::Error;

#[derive(Clone, Copy)]
pub(crate) struct Bytes<'a> {
    data: &'a [u8],
    /// Name of the structure being read, for error messages.
    what: &'static str,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, what }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn slice(&self, off: usize, len: usize) -> Result<&'a [u8], Error> {
        off.checked_add(len)
            .and_then(|end| self.data.get(off..end))
            .ok_or(Error::Truncated { what: self.what, offset: off, len })
    }

    /// A sub-view starting at `off` and running to the end of the data.
    pub fn from(&self, off: usize) -> Result<Bytes<'a>, Error> {
        let data = self.data.get(off..).ok_or(Error::Truncated { what: self.what, offset: off, len: 0 })?;
        Ok(Bytes { data, what: self.what })
    }

    pub fn with_what(self, what: &'static str) -> Self {
        Bytes { what, ..self }
    }

    pub fn u8(&self, off: usize) -> Result<u8, Error> {
        Ok(self.slice(off, 1)?[0])
    }

    pub fn u16(&self, off: usize) -> Result<u16, Error> {
        let b = self.slice(off, 2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&self, off: usize) -> Result<u32, Error> {
        let b = self.slice(off, 4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn array<const N: usize>(&self, off: usize) -> Result<[u8; N], Error> {
        Ok(self.slice(off, N)?.try_into().unwrap())
    }
}
