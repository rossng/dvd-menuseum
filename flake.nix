{
  description = "dvd-menuseum: DVD-Video menu engine and players";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            # Channel and wasm32 target come from rust-toolchain.toml.
            ((pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
              extensions = [
                "rust-src"
                "rust-analyzer"
                "clippy"
                "rustfmt"
              ];
            })

            # oracle/build.sh compiles .ref/libdvdnav against libdvdread.
            pkgs.pkg-config
            pkgs.libdvdread

            # scripts/: bundles.py, fetch_discs.py, difftest.py, looks.py, notices.py, dev.sh
            pkgs.python3
            pkgs.zstd
            pkgs.curl
            pkgs.git-lfs

            # web/, scripts/wasm-smoke.mjs and site-smoke.mjs
            pkgs.nodejs

            # Reference decoder for checking the media crate's output.
            pkgs.ffmpeg
          ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
