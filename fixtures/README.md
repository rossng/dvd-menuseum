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

| Slug | System | Licence | Credit | Why it's here |
|---|---|---|---|---|
| `elephants-dream-pal` | PAL | film CC BY 2.5 | Orange Open Movie Project, Blender Foundation and Netherlands Media Art Institute, 2006 | Professionally authored; 9 VTSs; ~180 MB motion menu |
| `elephants-dream-ntsc` | NTSC | film CC BY 2.5 | as above | Same disc for NTSC |
| `sita-sings-the-blues-pal` | PAL | film CC BY-SA 3.0 / CC0 | Nina Paley | Similar structure to Elephants Dream (5 VTSs, ~190 MB motion menu) |
| `nina-vision` | NTSC | CC BY-SA 3.0 | Nina Paley (animation), Terry Hancock (DVD) | Hand-authored; 125 MB looping VMGM motion menu |
| `av-calibration-pal` | PAL | CC BY-NC-SA 3.0 | Shaul Livshits | Test-pattern disc; no VMGM menu VOB |
| `av-calibration-ntsc` | NTSC | CC BY-NC-SA 3.0 | Shaul Livshits | Same disc for NTSC |

Credits are as given on the archive.org items (`archive_id` in `discs.json`). The Elephants Dream and Sita uploads carry no licence tag of their own. The licences above are those of the films, which their makers released freely.

