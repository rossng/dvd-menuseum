//! DVD-Video LPCM audio decoder, in the style of the oxideav codec crates:
//! [`register_codecs`] adds it to an `oxideav_core::CodecRegistry` under the
//! codec id `"pcm_dvd"`, and it implements `oxideav_core::Decoder`.
//!
//! # Packets
//!
//! LPCM travels in private_stream_1 substreams `0xa0..=0xa7`. After the
//! substream id each PES payload has a 3-byte private header (frame count,
//! first-access-unit pointer) followed by the 3-byte audio header this
//! decoder parses:
//!
//! | byte | bits | field |
//! |------|------|-------|
//! | 0 | 7 | emphasis |
//! | 0 | 6 | mute |
//! | 0 | 4..0 | frame number |
//! | 1 | 7..6 | quantization: 0 = 16, 1 = 20, 2 = 24 bits |
//! | 1 | 5..4 | sample rate: 0 = 48 kHz, 1 = 96 kHz |
//! | 1 | 2..0 | channels - 1 |
//! | 2 | 7..0 | dynamic range control (0x80 = none) |
//!
//! Each [`Packet`] given to the decoder is that audio header followed by
//! the samples, i.e. the PES payload minus the substream id and the 3-byte
//! private header.
//!
//! # Sample layout
//!
//! 16-bit samples are big-endian, interleaved by channel. 20- and 24-bit
//! samples are coded in groups of four consecutive interleaved samples (a
//! channel pair over two sample frames; two samples for mono): first the
//! upper 16 bits of each, big-endian, then the remaining low bits of each in
//! the same order (two nibbles per byte for 20 bits, a byte each for 24).
//! This matches ffmpeg's `pcm_dvd`, bit for bit; for more than two channels
//! it has only been checked against ffmpeg, not real discs. A group can
//! straddle packets, so leftover bytes are carried over.
//!
//! # Output
//!
//! Interleaved `SampleFormat::S32` frames: every sample is left-justified
//! in an `i32`, whatever its coded precision.

use std::collections::VecDeque;

use oxideav_core::{
    AudioFrame, CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry, Decoder, Error, Frame,
    Packet, Result, SampleFormat,
};

pub const CODEC_ID: &str = "pcm_dvd";

/// The per-packet LPCM audio header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub emphasis: bool,
    pub mute: bool,
    pub frame_number: u8,
    /// Coded bits per sample: 16, 20 or 24.
    pub bits: u8,
    pub sample_rate: u32,
    pub channels: u8,
    /// Dynamic range control byte (0x80 means no gain change).
    pub dynamic_range: u8,
}

impl Header {
    pub const LEN: usize = 3;

    pub fn parse(b: &[u8]) -> Result<Header> {
        let b: &[u8; 3] = b.get(..3).and_then(|s| s.try_into().ok()).ok_or_else(|| Error::invalid("pcm_dvd: packet shorter than its header"))?;
        let bits = match b[1] >> 6 {
            0 => 16,
            1 => 20,
            2 => 24,
            _ => return Err(Error::invalid("pcm_dvd: reserved quantization")),
        };
        let sample_rate = match (b[1] >> 4) & 3 {
            0 => 48_000,
            1 => 96_000,
            _ => return Err(Error::invalid("pcm_dvd: reserved sample rate")),
        };
        Ok(Header {
            emphasis: b[0] & 0x80 != 0,
            mute: b[0] & 0x40 != 0,
            frame_number: b[0] & 0x1f,
            bits,
            sample_rate,
            channels: (b[1] & 7) + 1,
            dynamic_range: b[2],
        })
    }

    /// Bytes per coded group, and the number of samples it holds.
    fn group(&self) -> (usize, usize) {
        let ch = self.channels as usize;
        let n = if ch == 1 { 2 } else { 4 };
        match self.bits {
            16 => (2 * ch, ch),
            20 => (2 * n + n / 2, n),
            _ => (3 * n, n),
        }
    }
}

/// Decode whole groups from `data` into left-justified i32 samples.
fn decode_groups(h: &Header, data: &[u8], out: &mut Vec<i32>) -> usize {
    let (group_len, samples) = h.group();
    let groups = data.len() / group_len;
    for g in data[..groups * group_len].chunks_exact(group_len) {
        match h.bits {
            16 => out.extend(g.chunks_exact(2).map(|s| (i16::from_be_bytes([s[0], s[1]]) as i32) << 16)),
            20 => {
                let (hi, lo) = g.split_at(samples * 2);
                out.extend(hi.chunks_exact(2).enumerate().map(|(i, s)| {
                    let nibble = if i % 2 == 0 { lo[i / 2] >> 4 } else { lo[i / 2] & 0x0f };
                    ((i16::from_be_bytes([s[0], s[1]]) as i32) << 16) | ((nibble as i32) << 12)
                }));
            }
            _ => {
                let (hi, lo) = g.split_at(samples * 2);
                out.extend(
                    hi.chunks_exact(2)
                        .zip(lo)
                        .map(|(s, &l)| ((i16::from_be_bytes([s[0], s[1]]) as i32) << 16) | ((l as i32) << 8)),
                );
            }
        }
    }
    groups * group_len
}

pub struct LpcmDecoder {
    codec_id: CodecId,
    /// Format of the bytes in `carry`.
    header: Option<Header>,
    /// Bytes of an incomplete group left over from the previous packet.
    carry: Vec<u8>,
    pending: VecDeque<AudioFrame>,
    eof: bool,
}

impl LpcmDecoder {
    pub fn new() -> Self {
        LpcmDecoder { codec_id: CodecId::new(CODEC_ID), header: None, carry: vec![], pending: VecDeque::new(), eof: false }
    }

    /// The format of the most recent packet, once one has been decoded.
    pub fn header(&self) -> Option<Header> {
        self.header
    }
}

impl Default for LpcmDecoder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(LpcmDecoder::new()))
}

/// Register the decoder under the `"pcm_dvd"` codec id.
pub fn register_codecs(reg: &mut CodecRegistry) {
    let info = CodecInfo::new(CodecId::new(CODEC_ID))
        .capabilities(CodecCapabilities::audio(CODEC_ID).with_decode().with_lossy(false))
        .decoder(make_decoder);
    reg.register(info);
}

impl Decoder for LpcmDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let h = Header::parse(&packet.data)?;
        let format = |h: &Header| (h.bits, h.sample_rate, h.channels);
        if self.header.as_ref().map(format) != Some(format(&h)) {
            // The format changed: a partial group of the old format is useless.
            self.carry.clear();
        }
        self.header = Some(h);
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(&packet.data[Header::LEN..]);
        let mut samples = Vec::with_capacity(data.len() / 2);
        let used = decode_groups(&h, &data, &mut samples);
        self.carry = data[used..].to_vec();
        if h.mute {
            samples.fill(0);
        }
        if !samples.is_empty() {
            self.pending.push_back(AudioFrame {
                samples: (samples.len() / h.channels as usize) as u32,
                pts: packet.pts,
                data: vec![samples.iter().flat_map(|s| s.to_le_bytes()).collect()],
            });
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(f) = self.pending.pop_front() {
            return Ok(Frame::Audio(f));
        }
        if self.eof {
            return Err(Error::Eof);
        }
        Err(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.header = None;
        self.carry.clear();
        self.pending.clear();
        self.eof = false;
        Ok(())
    }
}

/// The sample format every decoded frame uses.
pub const OUTPUT_FORMAT: SampleFormat = SampleFormat::S32;

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::TimeBase;

    fn packet(header: [u8; 3], samples: &[u8]) -> Packet {
        let mut data = header.to_vec();
        data.extend_from_slice(samples);
        Packet::new(0, TimeBase::new(1, 90_000), data)
    }

    fn decode(dec: &mut LpcmDecoder, p: Packet) -> Vec<i32> {
        dec.send_packet(&p).unwrap();
        let mut out = vec![];
        while let Ok(Frame::Audio(f)) = dec.receive_frame() {
            out.extend(f.data[0].chunks_exact(4).map(|b| i32::from_le_bytes(b.try_into().unwrap())));
        }
        out
    }

    #[test]
    fn parses_header() {
        let h = Header::parse(&[0x45, 0b0101_0101, 0x80]).unwrap();
        assert_eq!(
            h,
            Header { emphasis: false, mute: true, frame_number: 5, bits: 20, sample_rate: 96_000, channels: 6, dynamic_range: 0x80 }
        );
        assert!(Header::parse(&[0, 0b1100_0000, 0]).is_err());
    }

    #[test]
    fn sixteen_bit_stereo() {
        let mut d = LpcmDecoder::new();
        let s = decode(&mut d, packet([0, 0b0000_0001, 0x80], &[0x12, 0x34, 0xff, 0xfe]));
        assert_eq!(s, vec![0x1234 << 16, -2 << 16]);
    }

    #[test]
    fn twenty_four_bit_stereo_group() {
        // Samples (L0, R0, L1, R1) = 0x123456, 0xabcdef, 0x000001, 0x7fffff.
        let g = [0x12, 0x34, 0xab, 0xcd, 0x00, 0x00, 0x7f, 0xff, 0x56, 0xef, 0x01, 0xff];
        let mut d = LpcmDecoder::new();
        let s = decode(&mut d, packet([0, 0b1000_0001, 0x80], &g));
        assert_eq!(s, vec![0x12345600, 0xabcdef00u32 as i32, 0x00000100, 0x7fffff00]);
    }

    #[test]
    fn twenty_four_bit_groups_are_four_interleaved_samples() {
        // 4 channels, one sample frame pair: 8 samples in two groups of four.
        // Sample k (interleaved order) is 0x0k0k0k.
        let mut g = vec![];
        for group in 0..2 {
            for k in group * 4..group * 4 + 4 {
                g.extend_from_slice(&[k as u8, k as u8]);
            }
            for k in group * 4..group * 4 + 4 {
                g.push(k as u8);
            }
        }
        let mut d = LpcmDecoder::new();
        let s = decode(&mut d, packet([0, 0b1000_0011, 0x80], &g));
        let want: Vec<i32> = (0..8).map(|k| (k << 24) | (k << 16) | (k << 8)).collect();
        assert_eq!(s, want);
    }

    #[test]
    fn twenty_bit_group_straddles_packets() {
        // Mono 20-bit: two samples 0x12345 and 0xfedcb -> hi words, then nibbles 5 and b.
        let g = [0x12, 0x34, 0xfe, 0xdc, 0x5b];
        let mut d = LpcmDecoder::new();
        assert!(decode(&mut d, packet([0, 0b0100_0000, 0x80], &g[..3])).is_empty());
        let s = decode(&mut d, packet([1, 0b0100_0000, 0x80], &g[3..]));
        assert_eq!(s, vec![0x12345000, 0xfedcb000u32 as i32]);
    }
}
