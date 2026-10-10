# Fixtures

`discs.json` lists freely licensed DVD-Video images on archive.org. The data itself is gitignored. Fetch it with:

```sh
scripts/fetch_discs.py            # menu files only (~600 MB), via HTTP range reads
scripts/fetch_discs.py --full     # + whole disc images (~21 GB)
scripts/fetch_discs.py nina-vision av-calibration-ntsc   # a subset
```

Layout per disc:

```
fixtures/discs/<slug>/
  VIDEO_TS/      VIDEO_TS.{IFO,BUP,VOB}, VTS_nn_0.{IFO,BUP,VOB}: the menu data
  listing.json   every file in VIDEO_TS/ with size and starting LBA
  SHA256SUMS     checksums of the menu files
  disc.iso       full image (--full); input for the strip pipeline and the libdvdnav oracle
```

The menu-only `VIDEO_TS/` is extracted independently of `disc.iso`, so it doubles as a reference for what the stripper must preserve.

## Menu bundles

`bundles/<slug>.tar.zst` are the discs' menu bundles (`menuseum bundle`: IFOs and menu VOBs, no film), stored in Git LFS as zstd archives. Menus often repeat the same encoded video, which zstd's long mode finds: 670 MB of bundles take 458 MB. `scripts/bundles.py` extracts them to `bundles/<slug>/` (gitignored), where the player and tests read them; `scripts/dev.sh` runs it. It re-extracts only an archive that changed.

```sh
scripts/bundles.py                       # extract missing or stale bundles
menuseum bundle fixtures/discs/<slug>/disc.iso --out fixtures/bundles/<slug>
scripts/bundles.py pack <slug>           # then commit fixtures/bundles/<slug>.tar.zst
scripts/looks.py                         # and web/src/data/looks.json
```

Packing is reproducible, so an unchanged bundle repacks to the identical archive. `scripts/looks.py` samples each disc's look on the site from its bundle (its label's colours, and the shape of its first picture) with the `menuseum` CLI, and re-samples a disc whose archive or `menu` options change; CI checks the looks are current.

## Discs

`discs.json` is the one list of discs, in the order the site's shelf shows them. Each has the licence its menus keep (`license`: the film's, which its maker released freely), the attribution that asks for (`credit`), where it comes from (`archive_id`, the archive.org item, and `image`), and why it's here (`notes`, which also say where an upload is tagged with a different licence or none). Credits are as given on the archive.org items. The site's title, maker, year, `part` and `blurb` for each disc are there too. A disc with `"shelf": false` stays a fixture but is left off the site, which has to fit in GitHub Pages' 1 GB.

The menu bundles in `bundles/` are adapted from these discs (stripped of their titles) and keep each disc's licence. The `av-calibration-*` bundles are NonCommercial and ShareAlike; the `bloodspell-*` bundles are NonCommercial.
