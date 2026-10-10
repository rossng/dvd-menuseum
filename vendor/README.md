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
- `forming_predictions`: a prediction block whose reads, half-pel neighbours
  included, all lie inside the reference plane or field is formed a row at
  a time from the plane, rather than sample by sample through the
  pad-to-edge reader. Blocks reaching past an edge take the upstream path.
- `idct::idct_8x8`: performs the same floating-point products and sums as
  upstream's `idct_candidate_f64` (in the same order for every output, so
  the result is bit-identical), but accumulates a row of outputs at a time,
  skips zero coefficients and all-zero rows, and rounds without wasm's call
  to `round`. A test compares it with upstream on every single-coefficient
  block and 200,000 random ones.
- `examples/stream_decode.rs`: decodes an elementary stream through `StreamDecoder`
  in 2 KB chunks, for comparison against `decode_to_yuv` and ffmpeg.

`Cargo.lock`, `.cargo_vcs_info.json` and `Cargo.toml.orig` from the published crate
are omitted.
