//! Lookup tables for the longest-first VLC walks (local change for DVD
//! Menuseum).
//!
//! Several Annex B decoders walk their table longest codeword first,
//! peeking each width in turn and comparing it with every row of that
//! width. A [`LongestFirst`] table records, for every window of the
//! longest width, which row that walk would match, so a decoder can look
//! the row up with one peek. It answers only when the whole window is
//! available, which is when every peek of the walk would succeed; near
//! the end of the data, or when no row matches, the caller walks as
//! before.

use oxideav_core::bits::BitReader;

/// For every `width`-bit window, the index of the row the walk matches.
pub(crate) struct LongestFirst {
    width: u32,
    rows: Vec<u16>,
}

const NO_ROW: u16 = u16::MAX;

impl LongestFirst {
    /// `codes` are the table's `(code, bits)` rows in table order and
    /// `widths` the widths the walk tries, longest first.
    pub(crate) fn build(codes: &[(u16, u8)], widths: &[u8]) -> Self {
        let width = u32::from(*widths.iter().max().expect("a table has rows"));
        let rows = (0..1u32 << width)
            .map(|window| {
                for &w in widths {
                    let candidate = (window >> (width - u32::from(w))) as u16;
                    if let Some(i) = codes.iter().position(|&(code, bits)| bits == w && code == candidate) {
                        return i as u16;
                    }
                }
                NO_ROW
            })
            .collect();
        LongestFirst { width, rows }
    }

    /// The index of the row the walk would match at `br`, without
    /// consuming it, or `None` if the caller must walk.
    #[inline]
    pub(crate) fn find(&self, br: &mut BitReader<'_>) -> Option<usize> {
        if br.bits_remaining() < u64::from(self.width) {
            return None;
        }
        let window = br.peek_u32(self.width).ok()?;
        match self.rows[window as usize] {
            NO_ROW => None,
            i => Some(usize::from(i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_the_walk_finds() {
        // `1`, `01`, `001` and `0001`, plus a 3-bit `011` that the walk
        // tries before `01`, and a duplicate `001` that it never reaches.
        let codes = [(0b1, 1), (0b01, 2), (0b001, 3), (0b0001, 4), (0b011, 3), (0b001, 3)];
        let lut = LongestFirst::build(&codes, &[4, 3, 2, 1]);
        let find = |bytes: &[u8]| lut.find(&mut BitReader::new(bytes));
        assert_eq!(find(&[0b1000_0000]), Some(0));
        assert_eq!(find(&[0b0100_0000]), Some(1));
        assert_eq!(find(&[0b0110_0000]), Some(4));
        assert_eq!(find(&[0b0010_0000]), Some(2));
        assert_eq!(find(&[0b0001_0000]), Some(3));
        assert_eq!(find(&[0b0000_1000]), None);
    }
}
