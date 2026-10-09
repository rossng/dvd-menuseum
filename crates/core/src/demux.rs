//! Minimal MPEG-2 program stream demuxing of 2048-byte VOB sectors.

/// One PES packet's payload from a pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub stream_id: u8,
    /// For private_stream_1 (0xbd): the first payload byte (0x20..=0x3f subpicture,
    /// 0x80.. AC-3, 0xa0.. LPCM, ...). Not included in `payload`.
    pub substream: Option<u8>,
    /// Presentation time stamp (90 kHz), if present.
    pub pts: Option<u32>,
    pub payload: &'a [u8],
}

fn pts(b: &[u8]) -> u32 {
    // 33-bit PTS; DVD timestamps fit in 32 bits in practice.
    let v = ((b[0] as u64 >> 1) & 7) << 30
        | (b[1] as u64) << 22
        | ((b[2] as u64) >> 1) << 15
        | (b[3] as u64) << 7
        | (b[4] as u64) >> 1;
    v as u32
}

/// Split a pack into its PES packets. Malformed data ends the list early.
pub fn packets(sector: &[u8]) -> Vec<Packet<'_>> {
    let mut out = vec![];
    if sector.len() < 14 || sector[0..4] != [0, 0, 1, 0xba] {
        return out;
    }
    // MPEG-2 pack header: 14 bytes plus stuffing; MPEG-1: 12 bytes.
    let mut p = if sector[4] & 0xc0 == 0x40 { 14 + (sector[13] & 7) as usize } else { 12 };
    while p + 6 <= sector.len() && sector[p..p + 3] == [0, 0, 1] {
        let id = sector[p + 3];
        let len = u16::from_be_bytes([sector[p + 4], sector[p + 5]]) as usize;
        let body_start = p + 6;
        let end = (body_start + len).min(sector.len());
        p = body_start + len;
        if id == 0xbb || id == 0xbe || id == 0xbf {
            // System header, padding, private_stream_2 (NAV): no PES header extension.
            if id == 0xbf {
                out.push(Packet { stream_id: id, substream: None, pts: None, payload: &sector[body_start..end] });
            }
            continue;
        }
        let body = &sector[body_start..end];
        if body.len() < 3 || body[0] & 0xc0 != 0x80 {
            continue; // not an MPEG-2 PES header
        }
        let header_len = body[2] as usize;
        let pts = if body[1] & 0x80 != 0 && body.len() >= 8 { Some(pts(&body[3..8])) } else { None };
        let Some(mut payload) = body.get(3 + header_len..) else { continue };
        let mut substream = None;
        if id == 0xbd {
            let Some((&sub, rest)) = payload.split_first() else { continue };
            substream = Some(sub);
            payload = rest;
        }
        out.push(Packet { stream_id: id, substream, pts, payload });
    }
    out
}

/// Reassembles subpicture units, which span several packets: the first two
/// bytes of a unit give its total size.
#[derive(Clone, Debug, Default)]
pub struct SpuAssembler {
    buf: Vec<u8>,
    pts: Option<u32>,
}

impl SpuAssembler {
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Feed one packet's payload; returns a complete unit and its PTS when done.
    pub fn push(&mut self, payload: &[u8], pts: Option<u32>) -> Option<(Vec<u8>, Option<u32>)> {
        if self.buf.is_empty() {
            self.pts = pts;
        }
        self.buf.extend_from_slice(payload);
        if self.buf.len() < 2 {
            return None;
        }
        let size = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
        if size < 4 {
            self.buf.clear();
            return None;
        }
        if self.buf.len() >= size {
            let unit: Vec<u8> = self.buf.drain(..size).collect();
            self.buf.clear();
            return Some((unit, self.pts.take()));
        }
        None
    }
}
