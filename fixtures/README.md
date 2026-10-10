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
```

Packing is reproducible, so an unchanged bundle repacks to the identical archive.

## Discs

| Slug | System | Licence | Credit | Why it's here |
|---|---|---|---|---|
| `elephants-dream-pal` | PAL | film CC BY 2.5 | Orange Open Movie Project, Blender Foundation and Netherlands Media Art Institute, 2006 | Professionally authored; 9 VTSs; ~180 MB motion menu |
| `elephants-dream-ntsc` | NTSC | film CC BY 2.5 | as above | Same disc for NTSC |
| `sita-sings-the-blues-pal` | PAL | film CC BY-SA 3.0 / CC0 | Nina Paley | Similar structure to Elephants Dream (5 VTSs, ~190 MB motion menu) |
| `nina-vision` | NTSC | CC BY-SA 3.0 | Nina Paley (animation), Terry Hancock (DVD) | Hand-authored; 125 MB looping VMGM motion menu |
| `av-calibration-pal` | PAL | CC BY-NC-SA 3.0 | Shaul Livshits | Test-pattern disc; no VMGM menu VOB |
| `av-calibration-ntsc` | NTSC | CC BY-NC-SA 3.0 | Shaul Livshits | Same disc for NTSC |
| `big-buck-bunny-ntsc` | NTSC | CC BY 3.0 | Blender Foundation, 2008 | Official Peach DVD; 14 VTSs; ~82 MB motion menu |
| `big-buck-bunny-pal` | PAL | CC BY 3.0 | as above | Same disc for PAL; 11 VTSs |
| `sintel-pal` | PAL | film CC BY 3.0 | Blender Foundation, 2010 | Official Durian box set's film disc; 7 VTSs; ~86 MB motion menu |
| `sintel-ntsc` | NTSC | film CC BY 3.0 | as above | Same disc for NTSC |
| `valkaama-pal` | PAL | film CC BY-SA 3.0 | Tim Baumann, 2010 | Indie feature; single VTS, 28 MB menu |
| `bloodspell-1` | NTSC | CC BY-NC 2.5 Scotland | Strange Company, 2007 | Machinima feature; 3 VTSs |
| `bloodspell-2` | NTSC | CC BY-NC 2.5 Scotland | as above | Extras disc; 17 VTSs with small menus, so lots of navigation between them |

Credits are as given on the archive.org items (`archive_id` in `discs.json`). The Elephants Dream and Sita uploads carry no licence tag of their own, and the Sintel and Valkaama uploads are tagged with a 4.0 licence. The licences above are those of the films, which their makers released freely.

The menu bundles in `bundles/` are adapted from these discs (stripped of their titles) and keep each disc's licence. The `av-calibration-*` bundles are NonCommercial and ShareAlike; the `bloodspell-*` bundles are NonCommercial.
