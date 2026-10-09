//! MPEG-2 video decoding through the vendored oxideav decoder, and
//! conversion of decoded pictures to RGBA.

use oxideav_mpeg12video::video_sequence::{DecodedFrame, StreamDecoder};

/// A decoded picture in display order: planar 4:2:0 YCbCr.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
    /// Fields this picture is displayed for: 2, or 3 with repeat_first_field.
    pub fields: u32,
    /// Coded as an interlaced frame (two fields captured at different times).
    pub interlaced: bool,
    pub top_field_first: bool,
}

impl Picture {
    fn from_decoded(d: &DecodedFrame) -> Picture {
        let f = &d.frame;
        let (cw, ch) = f.visible_chroma_dims();
        Picture {
            width: f.width,
            height: f.height,
            y: f.y.packed_rect(f.width, f.height),
            cb: f.cb.packed_rect(cw, ch),
            cr: f.cr.packed_rect(cw, ch),
            fields: d.output_field_count(),
            interlaced: !d.progressive_frame,
            top_field_first: d.top_field_first,
        }
    }

    /// RGBA (BT.601 studio range). Interlaced pictures are deinterlaced by
    /// blending each line with its neighbours, which removes combing on
    /// motion at the cost of some vertical softness.
    pub fn to_rgba(&self) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let cw = w.div_ceil(2);
        let mut px = vec![255u8; w * h * 4];
        let luma = |row: usize, col: usize| -> i32 {
            if !self.interlaced || row == 0 || row + 1 >= h {
                self.y[row * w + col] as i32
            } else {
                // (above + 2 * this + below) / 4
                let (a, b, c) = (self.y[(row - 1) * w + col], self.y[row * w + col], self.y[(row + 1) * w + col]);
                (a as i32 + 2 * b as i32 + c as i32 + 2) / 4
            }
        };
        let clamp = |x: i32| x.clamp(0, 255) as u8;
        for row in 0..h {
            for col in 0..w {
                let y = (luma(row, col) - 16) * 298;
                let c = (row / 2) * cw + col / 2;
                let (u, v) = (self.cb[c] as i32 - 128, self.cr[c] as i32 - 128);
                let o = (row * w + col) * 4;
                px[o] = clamp((y + 409 * v + 128) >> 8);
                px[o + 1] = clamp((y - 100 * u - 208 * v + 128) >> 8);
                px[o + 2] = clamp((y + 516 * u + 128) >> 8);
            }
        }
        px
    }
}

/// Continuous MPEG-2 decoding. Decoding is never reset on jumps within a
/// disc: menu cells are often authored to be decoded straight after the
/// cell that preceded them.
#[derive(Default, Debug)]
pub struct VideoDecoder {
    dec: StreamDecoder,
    out: Vec<(DecodedFrame, u64)>,
}

impl VideoDecoder {
    /// Feed elementary-stream bytes; returns the pictures now displayable,
    /// each with the tag of the push its picture came from.
    pub fn push(&mut self, es: &[u8], tag: u64) -> Vec<(Picture, u64)> {
        self.dec.push(es, tag, &mut self.out);
        self.drain()
    }

    /// Decode everything buffered (end of a cell before a still, or end of stream).
    pub fn flush(&mut self) -> Vec<(Picture, u64)> {
        self.dec.flush(&mut self.out);
        self.drain()
    }

    fn drain(&mut self) -> Vec<(Picture, u64)> {
        self.out.drain(..).map(|(d, tag)| (Picture::from_decoded(&d), tag)).collect()
    }

    /// Pictures skipped because they failed to decode.
    pub fn errors(&self) -> u32 {
        self.dec.errors
    }
}
