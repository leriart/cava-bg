{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    naersk = {
      url = "github:nix-community/naersk";
      # Reuse the checkout we already evaluate instead of pulling a second,
      # unrelated nixpkgs just for naersk's own tests and fenix input.
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, naersk, flake-utils, ... }:
    let
      baseHmModule = import ./nix/home-module.nix;
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
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
          # naersk reads package.version from Cargo.toml, so Cargo.toml stays
          # the single source of truth for the version here as everywhere else.
          # It also reads Cargo.lock and fetches the smithay-client-toolkit git
          # dependency itself, pinned to the revision in the lock, so there is
          # no cargoLock.outputHashes (a buildRustPackage-only workaround).
          src = lib.cleanSource ./.;

          # `nix flake check --no-build` evaluates with a read-only store: a
          # path coerced to a store path is only computed, not written. naersk
          # discovers workspaces with
          #
          #     builtins.readDir "${root}/${dir}"
          #
          # so leaving `root` as a path makes the `--no-build` check fail with
          #
          #     error: path '/nix/store/...-source' is not valid
          #
          # before the build even starts. The flake's own source is already a
          # valid store path, so hand naersk that path as a string and avoid
          # coercing a fresh path while it discovers the workspace.
          root = toString ./.;

          buildInputs = runtimeDeps;

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
            installShellFiles
          ];

          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeDeps;

          # naersk has no `cargoFeatures` option: an unknown attribute is passed
          # straight to mkDerivation as an environment variable and never
          # reaches cargo. Append the feature to naersk's default build options
          # instead. This also covers the deps-only derivation that naersk
          # builds from a stub source tree.
          cargoBuildOptions =
            opts: opts ++ [ "--features=dbus-detection" ];

          postFixup = ''
            if [ -x "$out/bin/cava-bg" ]; then
              wrapProgram "$out/bin/cava-bg" \
                --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeDeps}" \
                --prefix PATH : "${lib.makeBinPath [ pkgs.cava pkgs.ffmpeg ]}"
            fi
          '';
          postInstall = ''
            COMPDIR="$NIX_BUILD_TOP/completions"
            if [ -d "$COMPDIR" ]; then
              installShellCompletion --bash --name cava-bg "$COMPDIR/cava-bg.bash"
              installShellCompletion --zsh --name cava-bg "$COMPDIR/_cava-bg"
              installShellCompletion --fish --name cava-bg "$COMPDIR/cava-bg.fish"
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
          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeDeps;
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

      homeModule = self.homeManagerModules.cava-bg;
    };
}
