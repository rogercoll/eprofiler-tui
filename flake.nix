# Based on https://github.com/NixOS/templates/tree/master/rust
{
  inputs = {
    naersk.url = "github:nix-community/naersk/master";
    nixpkgs.url = "https://channels.nixos.org/nixpkgs-unstable/nixexprs.tar.zst";
    utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      utils,
      naersk,
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        # zydis (via symblib) builds with cmake; eprofiler-proto needs protoc.
        nativeBuildInputs = with pkgs; [
          cmake
          protobuf
        ];
      in
      {
        # Needs the git submodules: `nix build '.?submodules=1'`
        packages.default = naersk-lib.buildPackage {
          src = ./.;
          inherit nativeBuildInputs;
        };
        devShells.default =
          with pkgs;
          mkShell {
            buildInputs = [
              cargo
              rustc
              rustfmt
              rustPackages.clippy
            ] ++ nativeBuildInputs;
            RUST_SRC_PATH = rustPlatform.rustLibSrc;
          };
      }
    );
}
