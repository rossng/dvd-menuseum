//! Subpicture unit (SPU) decoding and highlight compositing.
//!
//! A unit holds a 2-bit-per-pixel run-length coded bitmap (two interlaced
//! fields) and a list of display control sequences that set the display area,
//! the four colour (palette index) and contrast values, and timing.

use crate::nav::Hli;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spu {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// Pixel values 0..=3 (background, pattern, emphasis 1, emphasis 2), row-major.
    pub pixels: Vec<u8>,
    /// Palette index for each pixel value.
    pub color: [u8; 4],
    /// Contrast (0 transparent ..= 15 opaque) for each pixel value.
    pub contrast: [u8; 4],
    /// Display starts this many 90 kHz ticks after the unit's PTS.
    pub start: Option<u32>,
    /// Display stops this many ticks after the PTS (`None` = until replaced).
    pub stop: Option<u32>,
    /// Shown even when subtitles are off (menus use this).
    pub forced: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SpuError(pub &'static str);

/// Control sequence delays are in units of 1024 / 90000 s.
const DELAY_UNIT: u32 = 1024;

struct Nibbles<'a> {
    data: &'a [u8],
    /// Position in nibbles.
    pos: usize,
}

impl Nibbles<'_> {
    fn next(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 2)?;
        let n = if self.pos % 2 == 0 { byte >> 4 } else { byte & 0x0f };
        self.pos += 1;
        Some(n as u32)
    }

    fn align(&mut self) {
        self.pos += self.pos % 2;
    }
}

fn decode_field(data: &[u8], start: usize, width: usize, out: &mut [u8], rows: impl Iterator<Item = usize>) {
    let mut nib = Nibbles { data, pos: start * 2 };
    for row in rows {
        let line = &mut out[row * width..(row + 1) * width];
        let mut x = 0;
        while x < width {
            // Variable-length code: a value below 0x4, 0x10 or 0x40 needs another nibble.
            let Some(mut v) = nib.next() else { return };
            for threshold in [0x4, 0x10, 0x40] {
                if v >= threshold {
                    break;
                }
                let Some(n) = nib.next() else { return };
                v = (v << 4) | n;
            }
            let colour = (v & 3) as u8;
            let mut run = (v >> 2) as usize;
            if run == 0 {
                run = width - x; // to end of line
            }
            let end = (x + run).min(width);
            line[x..end].fill(colour);
            x = end;
        }
        nib.align();
    }
}

impl Spu {
    pub fn parse(unit: &[u8]) -> Result<Spu, SpuError> {
        if unit.len() < 4 {
            return Err(SpuError("unit too short"));
        }
        let size = u16::from_be_bytes([unit[0], unit[1]]) as usize;
        let unit = unit.get(..size).ok_or(SpuError("unit shorter than its size"))?;
        let mut dcsq = u16::from_be_bytes([unit[2], unit[3]]) as usize;
        let mut spu = Spu::default();
        let (mut top, mut bottom) = (None, None);
        let mut seen = std::collections::HashSet::new();
        // Walk the chain of display control sequences.
        while seen.insert(dcsq) {
            let date = u16::from_be_bytes([*unit.get(dcsq).ok_or(SpuError("bad DCSQ"))?, *unit.get(dcsq + 1).ok_or(SpuError("bad DCSQ"))?]) as u32;
            let next = u16::from_be_bytes([*unit.get(dcsq + 2).ok_or(SpuError("bad DCSQ"))?, *unit.get(dcsq + 3).ok_or(SpuError("bad DCSQ"))?]) as usize;
            let mut p = dcsq + 4;
            loop {
                let cmd = *unit.get(p).ok_or(SpuError("truncated DCSQ"))?;
                p += 1;
                let arg = |n: usize| unit.get(p..p + n).ok_or(SpuError("truncated command"));
                match cmd {
                    0x00 => {
                        spu.forced = true;
                        spu.start.get_or_insert(date * DELAY_UNIT);
                    }
                    0x01 => {
                        spu.start.get_or_insert(date * DELAY_UNIT);
                    }
                    0x02 => spu.stop = Some(date * DELAY_UNIT),
                    0x03 => {
                        let a = arg(2)?;
                        spu.color = [a[1] & 0x0f, a[1] >> 4, a[0] & 0x0f, a[0] >> 4];
                        p += 2;
                    }
                    0x04 => {
                        let a = arg(2)?;
                        spu.contrast = [a[1] & 0x0f, a[1] >> 4, a[0] & 0x0f, a[0] >> 4];
                        p += 2;
                    }
                    0x05 => {
                        let a = arg(6)?;
                        let x1 = ((a[0] as u16) << 4) | (a[1] as u16 >> 4);
                        let x2 = ((a[1] as u16 & 0x0f) << 8) | a[2] as u16;
                        let y1 = ((a[3] as u16) << 4) | (a[4] as u16 >> 4);
                        let y2 = ((a[4] as u16 & 0x0f) << 8) | a[5] as u16;
                        spu.x = x1;
                        spu.y = y1;
                        spu.width = x2.saturating_sub(x1) + 1;
                        spu.height = y2.saturating_sub(y1) + 1;
                        p += 6;
                    }
                    0x06 => {
                        let a = arg(4)?;
                        top = Some(u16::from_be_bytes([a[0], a[1]]) as usize);
                        bottom = Some(u16::from_be_bytes([a[2], a[3]]) as usize);
                        p += 4;
                    }
                    0x07 => {
                        // CHG_COLCON: per-region colour changes; skip its parameter block.
                        let a = arg(2)?;
                        p += u16::from_be_bytes([a[0], a[1]]) as usize;
                    }
                    0xff => break,
                    _ => return Err(SpuError("unknown SPU command")),
                }
            }
            if next == dcsq {
                break;
            }
            dcsq = next;
        }
        let (Some(top), Some(bottom)) = (top, bottom) else { return Err(SpuError("no pixel data offsets")) };
        let (w, h) = (spu.width as usize, spu.height as usize);
        if w == 0 || h == 0 || w > 1920 || h > 1152 {
            return Err(SpuError("implausible display area"));
        }
        spu.pixels = vec![0; w * h];
        decode_field(unit, top, w, &mut spu.pixels, (0..h).step_by(2));
        decode_field(unit, bottom, w, &mut spu.pixels, (1..h).step_by(2));
        Ok(spu)
    }
}

/// YCrCb palette entry (0x00YYCrCb) to RGB, ITU-R BT.601 studio range.
pub fn ycrcb_to_rgb(e: u32) -> [u8; 3] {
    let y = ((e >> 16) & 0xff) as f32 - 16.0;
    let cr = ((e >> 8) & 0xff) as f32 - 128.0;
    let cb = (e & 0xff) as f32 - 128.0;
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(1.164 * y + 1.596 * cr), c(1.164 * y - 0.813 * cr - 0.391 * cb), c(1.164 * y + 2.018 * cb)]
}

/// Which button rendering a highlight uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightMode {
    Selected,
    Activated,
}

/// A button rectangle with the colours and contrasts that replace the
/// subpicture's own inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
    pub color: [u8; 4],
    pub contrast: [u8; 4],
}

impl Highlight {
    fn contains(&self, x: u16, y: u16) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }
}

/// The highlight for `button` from the HLI's colour table, if it has one.
pub fn button_highlight(hli: &Hli, button: u8, mode: HighlightMode) -> Option<Highlight> {
    let b = hli.button(button)?;
    if b.coln == 0 {
        return None;
    }
    let word = hli.btn_colit[b.coln as usize - 1][(mode == HighlightMode::Activated) as usize];
    let (col, con) = ((word >> 16) as u16, word as u16);
    let nib = |v: u16, i: u32| ((v >> (4 * i)) & 0xf) as u8;
    Some(Highlight {
        x0: b.x_start,
        y0: b.y_start,
        x1: b.x_end,
        y1: b.y_end,
        color: [nib(col, 0), nib(col, 1), nib(col, 2), nib(col, 3)],
        contrast: [nib(con, 0), nib(con, 1), nib(con, 2), nib(con, 3)],
    })
}

/// Colour index and contrast of the pixel at (`col`, `row`) of `spu`.
fn pixel(spu: &Spu, col: usize, row: usize, highlight: Option<&Highlight>) -> (u8, u8) {
    let v = spu.pixels[row * spu.width as usize + col] as usize;
    match highlight {
        Some(h) if h.contains(spu.x + col as u16, spu.y + row as u16) => (h.color[v], h.contrast[v]),
        _ => (spu.color[v], spu.contrast[v]),
    }
}

/// A rendered subpicture: straight (non-premultiplied) RGBA covering the
/// subpicture's display area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// Render `spu` with `palette` (16 YCrCb entries), applying `highlight`.
pub fn render_overlay(spu: &Spu, palette: &[u32; 16], highlight: Option<&Highlight>) -> Overlay {
    let rgb: Vec<[u8; 3]> = palette.iter().map(|&e| ycrcb_to_rgb(e)).collect();
    let (w, h) = (spu.width as usize, spu.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        for col in 0..w {
            let (ci, alpha) = pixel(spu, col, row, highlight);
            let c = rgb[ci as usize & 0x0f];
            rgba[(row * w + col) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], alpha * 17]);
        }
    }
    Overlay { x: spu.x, y: spu.y, width: spu.width, height: spu.height, rgba }
}

/// Blend `spu` onto an RGB frame (`width` x `height`), applying `highlight`.
pub fn composite(
    frame: &mut [u8],
    width: usize,
    height: usize,
    spu: &Spu,
    palette: &[u32; 16],
    highlight: Option<&Highlight>,
) {
    let rgb: Vec<[u8; 3]> = palette.iter().map(|&e| ycrcb_to_rgb(e)).collect();
    for row in 0..(spu.height as usize).min(height.saturating_sub(spu.y as usize)) {
        for col in 0..(spu.width as usize).min(width.saturating_sub(spu.x as usize)) {
            let (ci, alpha) = pixel(spu, col, row, highlight);
            if alpha == 0 {
                continue;
            }
            let a = alpha as u32 * 17;
            let c = rgb[ci as usize & 0x0f];
            let o = ((spu.y as usize + row) * width + spu.x as usize + col) * 3;
            for k in 0..3 {
                frame[o + k] = ((c[k] as u32 * a + frame[o + k] as u32 * (255 - a)) / 255) as u8;
            }
        }
    }
}
