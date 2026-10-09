//! DVD audio decoding to stereo f32. Every codec goes through an
//! oxideav-core `Decoder`:
//!
//! | codec | stream | decoder | decoder output |
//! |-------|--------|---------|----------------|
//! | AC-3 | private_stream_1 0x80..=0x87 | oxideav-ac3 | S16, downmixed by the decoder |
//! | DTS | private_stream_1 0x88..=0x8f | oxideav-dts | S32P in DTS order (C, L, R, Ls, Rs, LFE) |
//! | LPCM | private_stream_1 0xa0..=0xa7 | menuseum-lpcm | S32, disc order |
//! | MPEG audio | stream ids 0xc0..=0xc7 | oxideav-mp2 | S16P |
//!
//! Each decoded frame is stamped with a presentation time: the PTS of the
//! packet in which the frame starts, or the previous frame's end.

use std::collections::VecDeque;

use oxideav_core::{CodecId, CodecParameters, CodecRegistry, Decoder, Frame, Packet, SampleFormat, TimeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Ac3,
    Dts,
    Lpcm,
    Mpeg,
}

impl Codec {
    /// The codec carried by a packet with this stream id and substream.
    pub fn of(stream_id: u8, substream: Option<u8>) -> Option<Codec> {
        match (stream_id, substream) {
            (0xbd, Some(0x80..=0x87)) => Some(Codec::Ac3),
            (0xbd, Some(0x88..=0x8f)) => Some(Codec::Dts),
            (0xbd, Some(0xa0..=0xa7)) => Some(Codec::Lpcm),
            (0xc0..=0xc7, None) => Some(Codec::Mpeg),
            _ => None,
        }
    }

    /// The (stream id, substream) of physical audio stream `n` coded with
    /// IFO audio format `format` (0 AC-3, 2/3 MPEG, 4 LPCM, 6 DTS).
    pub fn stream_for(format: u8, n: u8) -> Option<(u8, Option<u8>)> {
        let n = n & 7;
        match format {
            0 => Some((0xbd, Some(0x80 + n))),
            2 | 3 => Some((0xc0 + n, None)),
            4 => Some((0xbd, Some(0xa0 + n))),
            6 => Some((0xbd, Some(0x88 + n))),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::Ac3 => "AC-3",
            Codec::Dts => "DTS",
            Codec::Lpcm => "LPCM",
            Codec::Mpeg => "MPEG audio",
        }
    }
}

fn make_decoder(codec: Codec) -> Box<dyn Decoder> {
    let mut reg = CodecRegistry::new();
    let mut params = match codec {
        Codec::Ac3 => {
            oxideav_ac3::register_codecs(&mut reg);
            let mut p = CodecParameters::audio(CodecId::new("ac3"));
            p.channels = Some(2);
            p
        }
        Codec::Dts => {
            oxideav_dts::register_codecs(&mut reg);
            let mut p = CodecParameters::audio(CodecId::new("dts"));
            p.sample_format = Some(SampleFormat::S32P);
            p
        }
        Codec::Lpcm => {
            menuseum_lpcm::register_codecs(&mut reg);
            CodecParameters::audio(CodecId::new(menuseum_lpcm::CODEC_ID))
        }
        Codec::Mpeg => {
            oxideav_mp2::register_codecs(&mut reg);
            let mut p = CodecParameters::audio(CodecId::new("mp2"));
            p.channels = Some(2);
            p
        }
    };
    params.sample_rate = Some(48_000);
    reg.first_decoder(&params).expect("registered decoder")
}

/// Length of the frame at the start of `b`, if a valid header is there.
fn frame_len(codec: Codec, b: &[u8]) -> Option<usize> {
    match codec {
        Codec::Ac3 => {
            if b.len() < 7 || b[0] != 0x0b || b[1] != 0x77 {
                return None;
            }
            oxideav_ac3::syncinfo::parse(b).ok().map(|s| s.frame_length as usize).filter(|&n| n > 0)
        }
        Codec::Dts => {
            if b.len() < 12 || b[..4] != [0x7f, 0xfe, 0x80, 0x01] {
                return None;
            }
            // FTYPE(1) SHORT(5) CPF(1) NBLKS(7) FSIZE(14) after the sync word.
            let h = u64::from_be_bytes(b[4..12].try_into().ok()?);
            Some(((h >> 36) & 0x3fff) as usize + 1)
        }
        Codec::Mpeg => {
            const KBPS: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
            const RATE: [u32; 3] = [44100, 48000, 32000];
            let h = u32::from_be_bytes(b.get(..4)?.try_into().ok()?);
            let (sync, ver, layer, br, sr, pad) =
                (h >> 21, (h >> 19) & 3, (h >> 17) & 3, (h >> 12) & 15, (h >> 10) & 3, (h >> 9) & 1);
            if sync != 0x7ff || ver != 3 || layer != 2 || br == 0 || br == 15 || sr == 3 {
                return None;
            }
            Some((144 * KBPS[br as usize] * 1000 / RATE[sr as usize] + pad) as usize)
        }
        Codec::Lpcm => None,
    }
}

/// Decoded stereo audio with the presentation time of its first sample.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioChunk {
    /// The tag passed with the packet the chunk started in.
    pub tag: u64,
    /// 90 kHz presentation time of the first sample.
    pub pts: u32,
    pub sample_rate: u32,
    /// Interleaved stereo.
    pub samples: Vec<f32>,
}

/// One audio stream's decoder.
pub struct AudioDecoder {
    pub codec: Codec,
    dec: Box<dyn Decoder>,
    /// Frame-based codecs: bytes not yet split into frames.
    buf: Vec<u8>,
    /// (offset in `buf` where a packet began, its PTS, its tag).
    marks: VecDeque<(usize, Option<u32>, u64)>,
    /// Where the next chunk starts if it has no PTS of its own.
    next_pts: Option<u32>,
    sample_rate: u32,
    pub errors: u32,
}

impl std::fmt::Debug for AudioDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDecoder").field("codec", &self.codec).field("errors", &self.errors).finish_non_exhaustive()
    }
}

impl AudioDecoder {
    pub fn new(codec: Codec) -> Self {
        AudioDecoder {
            codec,
            dec: make_decoder(codec),
            buf: vec![],
            marks: VecDeque::new(),
            next_pts: None,
            sample_rate: 48_000,
            errors: 0,
        }
    }

    /// Feed one PES payload (after the substream id) with its PTS.
    pub fn push(&mut self, payload: &[u8], pts: Option<u32>, tag: u64, out: &mut Vec<AudioChunk>) {
        if self.codec == Codec::Lpcm {
            // 3-byte private header; the decoder parses the audio header after it.
            let Some(data) = payload.get(3..) else { return };
            if let Ok(h) = menuseum_lpcm::Header::parse(data) {
                self.sample_rate = h.sample_rate;
            }
            self.decode(data.to_vec(), pts, tag, out);
            return;
        }
        // AC-3 and DTS in private_stream_1 also carry a 3-byte private header.
        let data = if self.codec == Codec::Mpeg { payload } else { payload.get(3..).unwrap_or(&[]) };
        self.marks.push_back((self.buf.len(), pts, tag));
        self.buf.extend_from_slice(data);
        let mut off = 0;
        while off < self.buf.len() {
            match frame_len(self.codec, &self.buf[off..]) {
                Some(len) if off + len <= self.buf.len() => {
                    // The frame takes the PTS of the packet it starts in, if
                    // that packet's PTS hasn't been used by an earlier frame.
                    let mut frame_pts = None;
                    let mut frame_tag = self.marks.front().map_or(tag, |m| m.2);
                    while let Some(&(mo, mp, mt)) = self.marks.front() {
                        if mo > off {
                            break;
                        }
                        self.marks.pop_front();
                        frame_tag = mt;
                        if mp.is_some() {
                            frame_pts = mp;
                        }
                    }
                    let frame = self.buf[off..off + len].to_vec();
                    self.decode(frame, frame_pts, frame_tag, out);
                    off += len;
                }
                Some(_) => break,
                None if self.buf.len() - off < 12 => break,
                None => off += 1, // resync
            }
        }
        self.buf.drain(..off);
        for m in &mut self.marks {
            m.0 = m.0.saturating_sub(off);
        }
    }

    fn decode(&mut self, data: Vec<u8>, pts: Option<u32>, tag: u64, out: &mut Vec<AudioChunk>) {
        let pkt = Packet::new(0, TimeBase::new(1, 90_000), data);
        if self.dec.send_packet(&pkt).is_err() {
            self.errors += 1;
            return;
        }
        while let Ok(Frame::Audio(af)) = self.dec.receive_frame() {
            let samples = self.to_stereo(&af);
            let Some(start) = pts.or(self.next_pts) else {
                continue; // no timing yet: drop until a PTS arrives
            };
            let n = samples.len() / 2;
            self.next_pts = Some(start.wrapping_add((n as u64 * 90_000 / self.sample_rate as u64) as u32));
            out.push(AudioChunk { tag, pts: start, sample_rate: self.sample_rate, samples });
        }
    }

    fn to_stereo(&self, af: &oxideav_core::AudioFrame) -> Vec<f32> {
        let n = af.samples as usize;
        let planes = &af.data;
        let s16 = |p: &[u8], i: usize| i16::from_le_bytes([p[2 * i], p[2 * i + 1]]) as f32 / 32768.0;
        let s32 = |p: &[u8], i: usize| i32::from_le_bytes(p[4 * i..4 * i + 4].try_into().unwrap()) as f32 / 2_147_483_648.0;
        let mut out = Vec::with_capacity(2 * n);
        match self.codec {
            Codec::Ac3 => out.extend((0..2 * n).map(|i| s16(&planes[0], i))),
            Codec::Mpeg => {
                for i in 0..n {
                    let l = s16(&planes[0], i);
                    out.extend([l, planes.get(1).map_or(l, |p| s16(p, i))]);
                }
            }
            Codec::Dts => {
                let ch = planes.len();
                for i in 0..n {
                    let v = |c: usize| s32(&planes[c], i);
                    let (l, r) = match ch {
                        1 => (v(0), v(0)),
                        2 => (v(0), v(1)),
                        _ => {
                            let (c, l, r) = (v(0), v(1), v(2));
                            let (ls, rs) = if ch >= 5 { (v(3), v(4)) } else { (0.0, 0.0) };
                            ((l + 0.707 * c + 0.707 * ls) / 2.414, (r + 0.707 * c + 0.707 * rs) / 2.414)
                        }
                    };
                    out.extend([l, r]);
                }
            }
            Codec::Lpcm => {
                let ch = (planes[0].len() / 4 / n.max(1)).max(1);
                for i in 0..n {
                    let l = s32(&planes[0], i * ch);
                    out.extend([l, if ch > 1 { s32(&planes[0], i * ch + 1) } else { l }]);
                }
            }
        }
        out
    }
}
