{
  description = "cecs devshell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    templ.url = "github:a-h/templ";
  };

  outputs = inputs@{ self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) inputs.templ.overlays.default ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        templ = inputs.templ.packages.${system}.templ;
      in
      with pkgs;
      {
        devShells.default = mkShell {
          buildInputs = [
            (rust-bin.nightly.latest.default.override {
              extensions = [ "rust-src" "rust-analyzer" "rustfmt" ];
              targets = [ ];
            })
            cargo-nextest
            cargo-edit
            cargo-all-features
            just
            stdenv.cc.cc
          ];
          LD_LIBRARY_PATH = lib.makeLibraryPath [
          ];
        };
      }
    );
}

