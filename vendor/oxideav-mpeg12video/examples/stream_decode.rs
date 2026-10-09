//! Feed an elementary stream to `StreamDecoder` in 2 KB chunks and write
//! the frames as raw planar YUV (DVD Menuseum addition).
use std::io::Write;
use oxideav_mpeg12video::video_sequence::StreamDecoder;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let stream = std::fs::read(&a[1]).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a[2]).unwrap());
    let mut dec = StreamDecoder::new();
    let mut frames = vec![];
    let mut n = 0;
    let t = std::time::Instant::now();
    for chunk in stream.chunks(2048).chain(std::iter::once(&[][..])) {
        if chunk.is_empty() { dec.flush(&mut frames); } else { dec.push(chunk, &mut frames); }
        for f in frames.drain(..) {
            let fb = &f.frame;
            let (cw, ch) = fb.visible_chroma_dims();
            out.write_all(&fb.y.packed_rect(fb.width, fb.height)).unwrap();
            out.write_all(&fb.cb.packed_rect(cw, ch)).unwrap();
            out.write_all(&fb.cr.packed_rect(cw, ch)).unwrap();
            n += 1;
        }
    }
    eprintln!("{n} frames, {} errors, {:.1} fps", dec.errors, n as f64 / t.elapsed().as_secs_f64());
}
