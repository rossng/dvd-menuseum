# DVD Menuseum: player design

The DVD Menuseum is an archive of DVD menus that plays them in the browser as a DVD player would. It runs the disc's own navigation commands and decodes its original MPEG-2 video, audio and subpictures in WebAssembly.

## Prior art

| Project | What it is | Licence | Use to us |
|---|---|---|---|
| **libdvdnav / libdvdread** (VideoLAN; C) | The de facto DVD VM and navigation library behind VLC, mpv, Kodi and HandBrake. Carries 20 years of workarounds for broken discs. | GPL-2.0+ | **Reference oracle and the engine's model.** Our engine follows its VM and block loop closely, so `menuseum-engine` is GPL-2.0-or-later (see `LICENSING.md`). We diff our engine's event traces against it (`oracle/`). We don't link libdvdnav itself into the player. |
| **mpucoder DVD-Video info** (dvd.sourceforge.net/dvdinfo) | The community reverse-engineered spec: IFO layouts, PCI/DSI, VM opcodes, SPRMs, UOPs. | docs | **Primary spec.** |
| **Inside DVD-Video** (Wikibooks), rlaphoenix/dvd-specs | Prose explanations of the same material. | docs | Secondary spec. |
| **DVD.js** | libdvdread/libdvdnav ported to JS, with menus converted offline to WebM and baked button images. | GPL-3.0 | Shows the problem with converting offline: timed highlights, seamless transitions and odd command flows get lost. We play the original data instead. |
| **oxideav** (Rust; MIT) | A young pure-Rust media framework with MPEG-1/2 video, AC-3, MP2 and DTS decoders. | MIT | **Our decoders.** See below. |
| dvdauthor / spumux | DVD authoring tools. | GPL | Test-fixture generator. |

## Decisions

**Own navigation engine, validated against libdvdnav.** The VM is small and well documented, and owning it gives us a deterministic, step-driven core that's free of I/O and runs in wasm. Correctness comes from differential testing: libdvdnav, built from source with a deterministic `rand()` and clock, plays the same discs with the same scripted input, and the traces must match line for line, registers included. Where real players differ from libdvdnav, a quirks profile keeps the comparable behaviour available.

**Play the disc's original data.** Browsers can't decode MPEG-2 program streams, so we decode them ourselves in wasm rather than transcoding to H.264. That keeps the archive faithful to the disc. It also keeps decoding continuous across cells, as on a hardware player: menu transitions are often authored to follow the preceding cell, and decoding a cell on its own corrupts it. And it allows frame-accurate subpicture and highlight timing.

**Pure-Rust decoders:**

| Format | Decoder | Notes |
|---|---|---|
| MPEG-2 video | oxideav-mpeg12video, vendored in `vendor/` | Upstream decodes only a whole buffered stream and aborts on the first bad picture. Our vendored copy adds `StreamDecoder`, which decodes picture by picture and skips damaged pictures, and replaces its hottest table walks and per-sample loops with exact equivalents. About 300 fps for PAL interlaced in single-threaded wasm on an M-series Mac, against about 50 before those changes. |
| AC-3 | oxideav-ac3 | 35–53 dB against ffmpeg on real discs. One authoring tool's stream decodes noticeably differently; under investigation. |
| MPEG-1 Layer II | oxideav-mp2 | 60–80 dB against ffmpeg. |
| DTS | oxideav-dts | Correct on 16-bit-source DTS. Real 24-bit-source DTS-ES comes out about 256× too loud and clipped; under investigation. DVDs with DTS nearly always carry AC-3 too. |
| LPCM | `menuseum-lpcm` (ours) | Bit-exact against ffmpeg's `pcm_dvd` for 16/24-bit, 48/96 kHz, mono to 7.1. |
| Subpictures | `menuseum-core::spu` (ours) | RLE decode, palette and button highlight compositing. |

## Architecture

```
crates/
  core/    IFO + NAV parsing, VM, demux, subpicture decoding, sector cache,
           and the `Navigator` interface the player drives. No I/O, no clock.
  engine/  The navigation engine (libdvdnav-faithful; GPL-2.0-or-later).
  lpcm/    DVD LPCM decoder (oxideav-core Decoder).
  media/   Decodes the VOBUs the engine selects into video frames, audio and
           subpictures, and runs the real-time player: a `Navigator` + decoders +
           a presentation timeline, driven by host ticks and input.
  cli/     `menuseum`: info, disasm, trace, render, aspect, extract, bundle.
  wasm/    The player for the browser, plain C ABI, run in a Web Worker.
vendor/    oxideav-mpeg12video with StreamDecoder.
oracle/    libdvdnav built from source, with the same trace format.
web/       The website, an Astro site rendered statically from the catalog
           (scripts/site.py builds it and places the bundles beside it): the
           disc shelf, a player page per disc, one for the viewer's own DVD
           and the test harness, sharing src/player/player.js (video
           converted from YCbCr in a WebGL 2 shader, or RGBA from the
           decoder without it; overlay, Web Audio, input). Everything it
           loads comes from the site itself, under a content security policy;
           scripts/site-smoke.mjs checks every page in headless Chrome.
```

The engine is a VOBU-granular port of libdvdnav's VM and block loop, kept in its own crate behind the `Navigator` trait so that only it and the programs that link it are GPL. It asks a `NavSource` for NAV packs. A sector that hasn't arrived yet is *pending*, so the browser can stream a bundle with HTTP range requests. A missing title VOB set is *stripped*, and the engine skips its cells.

The player feeds each VOBU's sectors to the demuxer, then the decoders. It places frames and audio on a continuous presentation timeline: PTS values restart at cell boundaries, but the timeline doesn't. Each frame carries the overlay state, meaning the subpicture and button highlight, for its own moment. In the browser the player runs in a Worker; the page presents frames against the audio clock and forwards input.

## Menu bundle

A bundle is the disc with its film removed:

| Path | Contents |
|---|---|
| `VIDEO_TS/VIDEO_TS.IFO`, `VTS_nn_0.IFO` | verbatim |
| `VIDEO_TS/VIDEO_TS.VOB`, `VTS_nn_0.VOB` | menu VOB sets, verbatim |
| `bundle.json` | manifest for the web player: files and sizes, video format |

Title VOB sets (`VTS_nn_1..9.VOB`, the film) are left out. Their cells play as a short "not included" placeholder, so post-commands still run and menus still return. Menu VOBs are kept whole, so the bundle plays exactly as the disc's menus do.

That makes bundles large: menu video is ordinary DVD-bitrate MPEG-2, and discs with motion menus carry 2–4 minutes of it (125–190 MB). Being already compressed, it barely compresses further. The exception is menus that repeat the same encoded video, which zstd's long mode collapses. Fixture bundles are committed as `.tar.zst` in Git LFS and extracted for use (`fixtures/README.md`). The player streams the extracted files with HTTP range requests.

## Testing

1. Unit tests for parsing, the VM, subpictures and LPCM.
2. **Differential testing against libdvdnav** (`scripts/difftest.py`): random input scripts, identical traces required.
3. Decoder checks against ffmpeg on real and generated streams: PSNR for video, SNR for audio, bit-exactness for LPCM.
4. Visual comparison against reference players (VLC, Apple DVD Player): planned.
