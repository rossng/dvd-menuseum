# Licensing

The DVD Menuseum is under the [Blue Oak Model License 1.0.0](LICENSE.md), except
for the parts listed below. The navigation engine is derived from libdvdnav, so
it and the programs that link it are under the GNU GPL, version 2 or (at your
option) any later version ([COPYING](COPYING)). It sits behind the `Navigator`
trait in `menuseum-core` so that nothing else needs it.

| Path | Licence | Why |
|---|---|---|
| everything not listed below | BlueOak-1.0.0 | |
| `crates/engine/` | GPL-2.0-or-later | follows libdvdnav's `vm.c`, `play.c`, `getset.c`, `vmget.c`, `dvdnav.c`, `highlight.c` and `searching.c` |
| `crates/cli/` | GPL-2.0-or-later | the `menuseum` binary links the engine |
| `crates/wasm/` | GPL-2.0-or-later | the browser player links the engine |
| `oracle/` | GPL-2.0-or-later | built against libdvdnav's internal headers |
| `vendor/oxideav-mpeg12video/` | MIT | third-party; our local changes are MIT too ([its LICENSE](vendor/oxideav-mpeg12video/LICENSE)) |
| `fixtures/bundles/*.tar.zst` | per disc, CC | disc content; see [fixtures/README.md](fixtures/README.md) |

`menuseum-core`, `menuseum-lpcm` and `menuseum-media` don't depend on the engine
(`menuseum-media` uses it only for `examples/play.rs`). A binary built from them
alone is BlueOak plus its MIT dependencies.

## Distributing the binaries

The `menuseum` CLI and `menuseum_wasm.wasm` are GPL-2.0-or-later as a whole.
Whoever distributes them must offer the corresponding source and pass on the
notices of what they contain: the licences in this repository and those of
the third-party crates compiled in (the oxideav crates, all Copyright (c) 2026
Karpelès Lab Inc., and a few of their dependencies, all used under MIT).
`scripts/notices.py` lists the player's from `cargo metadata` into
`web/src/data/notices.json`, and CI checks the list is current. The site shows
them on its notices page (`web/src/pages/notices.astro`) and in the Licences
dialog in every page's footer, which also links the source of the commit the
site was built from (`scripts/site.py` records it in the catalog).

Only the web player is distributed so far. A release of the `menuseum` binary
would need the same notices shipped beside it.

## Disc menus

The menu bundles keep each disc's Creative Commons licence (see
[fixtures/README.md](fixtures/README.md)). The site credits each disc as
`credit` in `web/discs.json`, links its licence, and says the menus are
adapted from the discs. The `av-calibration-*` discs are NonCommercial, so the
site must not carry advertising or be run commercially while it serves them.
