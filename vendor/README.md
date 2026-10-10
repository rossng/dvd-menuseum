# Vendored crates

## oxideav-mpeg12video 0.0.13

From crates.io (MIT, https://github.com/OxideAV/oxideav-mpeg12video). Local changes:

- `video_sequence::StreamDecoder`: an incremental decoder. Upstream decodes only a
  whole buffered elementary stream (`decode_video_sequence`) and aborts on the first
  picture that fails to decode. `StreamDecoder` takes bytes as they arrive, decodes
  each picture once the start code that ends it is in, emits frames in display order
  as soon as the reorder allows, and skips (counts) pictures that fail. Each push
  carries a tag, and every frame comes out with the tag of the push holding its
  picture, so callers can time frames by the access unit they came from.
- `mpeg2_dct_coeff`: DCT coefficient codewords are looked up in decode tables
  built from Tables B-14/B-15 on first use, instead of scanning every row of
  every codeword width for each coefficient (which was about 70% of decode
  time). The scan remains for the last few bits of a buffer. A test checks
  the tables against the scan for every 16-bit window.
- `examples/stream_decode.rs`: decodes an elementary stream through `StreamDecoder`
  in 2 KB chunks, for comparison against `decode_to_yuv` and ffmpeg.

`Cargo.lock`, `.cargo_vcs_info.json` and `Cargo.toml.orig` from the published crate
are omitted.
