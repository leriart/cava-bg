{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    let
      baseHmModule = import ./nix/home-module.nix;
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
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

        # nixpkgs' own Rust builder rather than naersk.
        #
        # naersk failed `nix flake check` with
        #     error: path '/nix/store/...-source' is not valid
        # inside its `findCargoTomls` helper, and it reproduces with a minimal
        # naersk flake that has nothing to do with this repository, so it is an
        # upstream problem rather than something in flake.nix. buildRustPackage
        # needs no extra flake input (naersk dragged in fenix and
        # rust-analyzer-src too) and understands Cargo.lock natively.
        cava-bg = pkgs.rustPlatform.buildRustPackage {
          pname = "cava-bg";
          # Cargo.toml stays the single source of truth for the version, as it
          # is everywhere else in the project.
          version = (lib.importTOML ./Cargo.toml).package.version;

          src = ./.;

          cargoLock =
            let
              lock = builtins.fromTOML (builtins.readFile ./Cargo.lock);
              gitDeps = builtins.filter
                (p: lib.hasPrefix "git+" (p.source or ""))
                lock.package;

              # A git dependency's source tree cannot be hashed by cargoLock, so
              # the hash is unavoidably a literal. The *key* is not: writing
              # "smithay-client-toolkit-0.20.0" here would duplicate the version
              # from Cargo.lock, and bumping the crate would leave the stale key
              # silently in place. The version is read from the lock file and
              # only the hash is recorded, keyed by crate name.
              hashes = {
                smithay-client-toolkit = "sha256-rFNSzm1003spYHdzlvCyb7PWGMOJibUq/Ek3F5HABkM=";
              };
            in
            {
              lockFile = ./Cargo.lock;
              outputHashes = builtins.listToAttrs (map
                (p: {
                  name = "${p.name}-${p.version}";
                  value = hashes.${p.name} or (throw
                    "Cargo.lock has a git dependency '${p.name}' with no recorded hash. "
                    + "Add   ${p.name} = \"sha256-...\";   to cargoLock.hashes in flake.nix. "
                    + "Nix prints the expected hash when the build fails.");
                })
                gitDeps);
            };

          buildFeatures = [ "dbus-detection" ];

          buildInputs = runtimeDeps;

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
            installShellFiles
          ];

          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeDeps;

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
