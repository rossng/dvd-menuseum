# Vendored crates

## oxideav-mpeg12video 0.0.13

From crates.io (MIT, https://github.com/OxideAV/oxideav-mpeg12video). Local changes:

- `video_sequence::StreamDecoder`: an incremental decoder. Upstream decodes only a
  whole buffered elementary stream (`decode_video_sequence`) and aborts on the first
  picture that fails to decode. `StreamDecoder` takes bytes as they arrive, decodes
  each picture once the start code that ends it is in, emits frames in display order
  as soon as the reorder allows, and skips (counts) pictures that fail.
- `examples/stream_decode.rs`: decodes an elementary stream through `StreamDecoder`
  in 2 KB chunks, for comparison against `decode_to_yuv` and ffmpeg.

`Cargo.lock`, `.cargo_vcs_info.json` and `Cargo.toml.orig` from the published crate
are omitted.
