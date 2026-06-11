{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    naersk.url = "github:nix-community/naersk";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, naersk, flake-utils, ... }:
    let
      baseHmModule = import ./nix/home-module.nix;
    in
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
          cava
        ];

        cava-bg = naersk-lib.buildPackage {
          name = "cava-bg";
          src = ./.;

          buildInputs = runtimeDeps;

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
            installShellFiles
          ];

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;

          cargoFeatures = [ "dbus-detection" ];

          postFixup = ''
            if [ -x "$out/bin/cava-bg" ]; then
              wrapProgram "$out/bin/cava-bg" \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeDeps}" \
                --prefix PATH : "${pkgs.lib.makeBinPath [ pkgs.cava pkgs.ffmpeg ]}"
            fi
          '';
          postInstall = ''
            COMPDIR="$NIX_BUILD_TOP/completions"
            if [ -d "$COMPDIR" ]; then
              installShellCompletion --bash --name cava-bg "$COMPDIR/cava-bg.bash"
              installShellCompletion --zsh --name _cava-bg "$COMPDIR/_cava-bg"
              installShellCompletion --fish --name cava-bg.fish "$COMPDIR/cava-bg.fish"
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
    )
    // {
      homeManagerModules.cava-bg-base = baseHmModule;

      homeManagerModules.cava-bg = { config, lib, pkgs, ... }: {
        imports = [ baseHmModule ];
        programs.cava-bg.package = lib.mkDefault (
          self.packages.${pkgs.stdenv.hostPlatform.system}.default
        );
      };

      homeManagerModule = self.homeManagerModules.cava-bg;
    };
}
