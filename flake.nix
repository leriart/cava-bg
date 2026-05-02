{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    naersk.url = "github:nix-community/naersk";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, naersk, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        rustSrc = pkgs.rust.packages.stable.rustPlatform.rustLibSrc;

        runtimeDeps = with pkgs; [
          wayland
          wayland-protocols
          libxkbcommon
          mesa
          libglvnd
          ffmpeg
          dbus
        ];

        cava-bg = naersk-lib.buildPackage {
          name = "cava-bg";
          src = ./.;

          buildInputs = runtimeDeps;

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
          ];

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;

          cargoFeatures = [ "dbus-detection" ];

          postFixup = ''
            if [ -x "$out/bin/cava-bg" ]; then
              wrapProgram "$out/bin/cava-bg" \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeDeps}"
            fi
          '';
        };
      in {
        packages.default = cava-bg;
        packages.cava-bg = cava-bg;
        defaultPackage = cava-bg;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ cava-bg ];
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;
          RUST_SRC_PATH = "${rustSrc}";
        };
      }
    );
}
