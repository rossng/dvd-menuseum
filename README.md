# DVD Menuseum

An archive of DVD menus that plays them in the browser as a DVD player would: <https://rossng.github.io/dvd-menuseum/>

It runs each disc's own navigation commands and decodes its original MPEG-2 video, audio and subpictures in WebAssembly, so highlights, transitions and odd command flows behave as they did on the disc. The discs are freely licensed DVD-Video images from archive.org, and only their menus are kept. The player can also play a DVD from the viewer's own files.

## Building

The Nix flake has the development shell (`nix develop`). The menu bundles are in Git LFS, and a clone doesn't fetch them, so fetch them first (see [fixtures/README.md](fixtures/README.md)):

```sh
git lfs pull --exclude=""     # ~670 MB; or --include one archive
scripts/dev.sh                # build the wasm player and serve the site on :8000
cargo test --workspace
```

[docs/design.md](docs/design.md) describes the player and how it's tested against libdvdnav.

## Licence

Blue Oak Model License 1.0.0, except for the navigation engine and what links it (the CLI, the wasm player and the oracle), which follow libdvdnav and are GPL-2.0-or-later. The disc menus keep their discs' Creative Commons licences. [LICENSING.md](LICENSING.md) has the details.
