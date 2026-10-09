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
| `vendor/oxideav-mpeg12video/` | MIT | third-party, with local changes ([its LICENSE](vendor/oxideav-mpeg12video/LICENSE)) |

`menuseum-core` doesn't depend on the engine.
