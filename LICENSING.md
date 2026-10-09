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
| `vendor/oxideav-mpeg12video/` | MIT | third-party, with local changes ([its LICENSE](vendor/oxideav-mpeg12video/LICENSE)) |

`menuseum-core`, `menuseum-lpcm` and `menuseum-media` don't depend on the engine
(`menuseum-media` uses it only for `examples/play.rs`). A binary built from them
alone is BlueOak plus its MIT dependencies.

## Distributing the binaries

The `menuseum` CLI is GPL-2.0-or-later as a whole.
Whoever distributes it must offer the corresponding source and pass on the
notices of what they contain: the licences in this repository and the MIT
notices of the oxideav crates (`oxideav-core`, `oxideav-ac3`, `oxideav-dts`,
`oxideav-mp2` and the vendored `oxideav-mpeg12video`, all Copyright (c) 2026
Karpelès Lab Inc.).
