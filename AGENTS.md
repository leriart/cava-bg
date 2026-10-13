# AGENTS.md

Guidance for AI agents (and humans) working in this repository.

## What this project is

**cava-bg** is a native Wayland audio visualizer. It renders audio-reactive bars as a
`wlr-layer-shell` **background** layer on top of the wallpaper, so nothing ever gets
a visible window. Rust, `wgpu` for rendering, MIT licensed.

It does **not** capture audio itself: it spawns the external `cava` binary and reads
its raw `16bit` stdout stream.

```
cava (subprocess)
  └─ raw u16 bars on stdout
       └─ reader thread: u16 -> f32 (0..1), packaged as CavaFramePacket
            └─ mpsc channel
                 └─ WaylandRenderer::draw()  builds triangle-list vertices on the CPU
                      └─ wgpu render pass per output, into a layer-shell surface
```

On top of that the daemon can hot-swap a gradient derived from the current wallpaper,
reveal a hidden "X-Ray" image through the bars, decode video for X-Ray/parallax
layers, and drive a mouse/audio-reactive parallax stack.

## Repository layout

```
.
├── AGENTS.md               this file
├── CONFIG.md               generated — do not edit
├── Cargo.toml              single source of truth for the version
├── install.sh              the only end-user installer; also used by Nix/packagers
├── build.rs                generates shell completions from src/cli.inc.rs
├── flake.nix / nix/        Nix packaging + Home Manager module
├── .github/workflows/      ci.yml (every push/PR) · release.yml (on main)
├── docs/
│   ├── INSTALL.md          long-form install paths
│   └── media/              screenshots and demos referenced by the README
├── scripts/
│   └── version.sh          sourced by every packaging script
├── packaging/
│   ├── build-all.sh        the single packaging entry point
│   ├── cava-bg.desktop     shared desktop entry (Exec=cava-bg gui)
│   ├── systemd/            cava-bg.service
│   ├── deb/                build-deb.sh · control · postinst
│   ├── rpm/                build-rpm.sh · cava-bg.spec
│   └── aur/                PKGBUILD.in · render-pkgbuild.sh
└── src/ · xtask/
```

**Where things belong.** There is exactly one way to do each job; do not add a
second script for something that already exists.

* Versions come from `scripts/version.sh` (which reads `Cargo.toml`), never from
  a literal.
* Packaging artifacts live under `packaging/`, split by target format.
* Documentation lives under `docs/`; the README is the index.
* Runtime code is strictly in `src/`; `xtask/` only generates `CONFIG.md`.

## Ground rules

1. **`Cargo.toml` is the single source of truth for the version.** Never hardcode a
   version anywhere else. Shell scripts and packaging templates must
   `source scripts/version.sh` and use `$(cava_bg_version)`. CI fails the build if a
   literal version appears in a `*.sh`, `*.spec` or `PKGBUILD.in`.
2. **`CONFIG.md` is generated.** Never edit it by hand — edit the doc comments on the
   structs in `src/app_config.rs`, then run `cargo xtask`.
3. **There is no `config.toml` in the repo.** A fresh config is rendered by
   `cava-bg __default-config`, which serializes `Config::default()`. Do not
   reintroduce a hand-written one: the old root-level file disagreed with the code
   defaults on `bar_width`, `max_bar_height`, `bar_alpha`, `use_gradient` and the
   palette, so every installed user got behaviour different from what `CONFIG.md`
   documents. CI fails if one reappears.
4. **Serialize configs with `app_config::serialize_config`, not
   `toml::to_string_pretty`.** The wrapper rounds `f32` widening noise
   (`0.1` → `0.10000000149011612`); every write path goes through it.
5. **Work happens on `dev`.** `main` is release-only. See *Branching and releases*.
6. **Every merge to `dev` must keep CI green** (fmt, clippy `-D warnings`, tests,
   `cargo xtask -- --check`, and the packaging job).
7. **Don't add dependencies without discussing it.** The tree is already ~1.5M lines
   of `Cargo.lock`; the binary links FFmpeg, wgpu, eframe and Wayland.
8. **Avoid logging inside `Deserialize`/`Serialize` impls on the config hot-reload
   path.** A bad value is re-parsed every second, so a per-parse `eprintln!` spams the
   daemon log forever. Prefer a warn-once guard (see
   `app_config::report_unknown_color_keys`).
9. **Internal back-compat fields must be `skip_serializing`.** Fields named
   `_legacy_*` exist only to read old configs; writing them back puts noise in every
   user file. They keep their `alias` for parsing.
10. **Code comments and log messages in English.** The codebase is English-only; a few
    Spanish strings are legacy and should not be copied.

## Commands

```bash
cargo build                 # debug build
cargo build --release       # LTO fat, codegen-units=1, panic=abort, stripped
cargo test --workspace      # unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo xtask                 # regenerate CONFIG.md
cargo xtask -- --check      # fail if CONFIG.md is stale (this is what CI runs)
```

System dependencies (Debian/Ubuntu):

```bash
apt-get install libwayland-dev libxkbcommon-dev libegl1-mesa-dev \
                libgl1-mesa-dev libdbus-1-dev pkg-config
```

`ffmpeg-next 9.0` needs the system FFmpeg 8/9 development headers. A mismatch surfaces
as `libavutil.so.60` / `libavutil.so.61` link errors at runtime — see
`install.sh:check_ffmpeg_linkage`.

### Running locally

You need a real Wayland session with a compositor that implements `wlr-layer-shell`
(Hyprland, Sway, river, …). X11 will not work.

```bash
cargo run --release -- on --debug     # foreground, logs to stderr
cargo run --release -- gui            # egui configurator
cargo run --release -- status
cargo run --release -- outputs
```

Logs go to `~/.config/cava-bg/daemon.log`; config to
`~/.config/cava-bg/config.toml`.

## Architecture

`src/` modules, roughly in dependency order:

| Module | Responsibility |
|---|---|
| `paths.rs` | **Canonical filesystem layout.** Every runtime path (pid, log, socket, presets, `runtime-outputs.json`) derives from the config file's directory via `RuntimePaths`. Add new runtime files here — never call `dirs::config_dir()` ad hoc. |
| `cli.inc.rs` | The `clap::Command` definition. `include!`d by both `src/cli.rs` and `build.rs` so generated shell completions can never drift from the parser. |
| `cli.rs` | Turns `ArgMatches` into the `Cli` struct + `Command` enum. `__default-config` prints `Config::default()` as TOML. |
| `app_config.rs` | The whole TOML schema. `Config` → `ConfigOverride` (all-`Option`) → per-output resolution with wildcard matching (`wildcard_matches`, `pattern_specificity`). Also holds the `xtask` doc-comment protocol. |
| `main.rs` | Process lifecycle: CLI dispatch, single-instance guard, pid files, log rotation, systemd `sd-notify` + watchdog, the `catch_unwind` keep-alive loop. |
| `supervisor.rs` | `--supervisor` mode: one child process per output so a GPU/decoder crash on one monitor cannot take down the others. |
| `wayland_renderer.rs` | The core. Owns the `cava` subprocess, the calloop event loop, `AppState`, per-output wgpu state, hot reload, and the visualizer vertex builders. |
| `parallax_system.rs` | Mouse/audio reactive parallax layer stack. `compute_transform` is the interesting function. |
| `wallpaper_detector.rs` | Finds the current wallpaper: 6 priority tiers (IPC → D-Bus → Ambxst QML → daemon config files → `/proc` scan → layer-shell background). 5 s result cache. |
| `wallpaper.rs` | Palette extraction (`color_thief`) + cross-frame palette smoothing + the wallpaper monitor threads. |
| `config_gui.rs` | egui/eframe configurator. Auto-saves with a 180 ms debounce and pushes live colour updates to the running daemon over a Unix datagram socket. |
| `video_decoder.rs` | FFmpeg decode into `VideoFrame`, with frame-skipping for A/V sync. |
| `bar_geometry.rs` | CPU triangle-list vertex generation. `vertices_per_bar` sizes the GPU buffer up front — **update it whenever you touch `build_bar`.** |
| `xray_animator.rs` | X-Ray intensity animation (audio-sync / wallpaper-phase modes). |
| `perf_monitor.rs` | Allocation-free frame-time sliding window. Feeds `parallax.performance.disable_under_load`. |
| `layer_system.rs` | **Dead code.** The original X-Ray fingerprint-mask engine, superseded by the shader approach. `LayerManager` has no call sites. |
| `layer_finder.rs` | Only `discover_parallax_layers` is used (by the GUI). |

Shaders live next to the code: `src/shader.wgsl` (bars + gradient + X-Ray reveal) and
`src/parallax_shader.wgsl`.

`xtask/` is a separate workspace member that parses `src/app_config.rs` with `syn` and
emits `CONFIG.md`.

## Config system

`~/.config/cava-bg/config.toml`, TOML, hot-reloaded by mtime every 1 s.

Resolution order per output (`Config::resolve_for_output`):

1. Start from the base `Config`.
2. Apply `[global]` (an all-`Option` `ConfigOverride`).
3. Apply the **best-matching** `[output."<key>"]` by specificity score — exact `name`
   (+1000) beats a wildcard, `connector` (+70) beats `index` (+40).
4. If the winning override has `enabled = false`, the output is skipped entirely.
5. `normalize_compat_fields()` reconciles legacy alias fields in both directions.

When you add a config field:

1. Add it to the right struct in `src/app_config.rs` with a `///` doc comment and a
   `#[serde(default = "default_x")]` or `#[serde(default)]`.
2. Add it to `ConfigOverride` too if it should be overridable per output.
3. Wire it into `normalize_compat_fields()` if it replaces a legacy field.
4. Wire it into `config_gui.rs` if users should see it.
5. Run `cargo xtask` and commit the regenerated `CONFIG.md`.

Use `#[cfg_attr(any(), config_doc(hide_defaults))]` or
`#[cfg_attr(any(), config_doc(skip))]` to control what `xtask` emits.

## Release process

```
feature branch ──▶ dev ──▶ (CI) ──▶ main ──▶ release.yml
                                            ├─ tag v<version>
                                            ├─ source + binary tarballs (+sha256)
                                            ├─ .deb   (Debian/Ubuntu)
                                            ├─ .rpm   (Fedora/openSUSE)
                                            ├─ GitHub Release
                                            └─ AUR PKGBUILD + .SRCINFO
```

* Work lands on **`dev`**.
* When `dev` is merged into **`main`**, `.github/workflows/release.yml` runs
  automatically. It reads the version from `Cargo.toml`, skips if the tag already
  exists, creates + pushes `v<version>`, builds every artifact, publishes the GitHub
  release, and (when the `aur` environment gate passes) pushes the rendered `PKGBUILD`
  and `.SRCINFO` to the AUR.
* Bump the release by editing `version` in `Cargo.toml` **only**, then land that on
  `dev` and merge to `main`.
* `packaging/build-all.sh` reproduces every artifact locally. It refuses to run on a
  dirty tree (`--allow-dirty` overrides) because a tarball from uncommitted files is
  not reproducible.

### AUR

The AUR repository is external and receives only `PKGBUILD` + `.SRCINFO`. Required
repository secrets:

| Secret | Purpose |
|---|---|
| `AUR_SSH_PRIVATE_KEY` | SSH key whose public half is registered at aur.archlinux.org |
| `AUR_SSH_KNOWN_HOSTS` | `aur.archlinux.org` host key line |

Both live on the `aur` GitHub **environment** so a maintainer can gate publishing.
Without them the `aur` job fails early with a clear error instead of silently skipping.
Local preview: `VERSION=0.2.6 ARCH=x86_64 bash packaging/aur/render-pkgbuild.sh`.

### Packaging layout

```
packaging/
├── build-all.sh             the single entry point: source + binary tarball,
│                            .deb, .rpm and the rendered AUR files
├── cava-bg.desktop          shared desktop entry (Exec=cava-bg gui)
├── systemd/cava-bg.service  systemd user unit, installed by deb and rpm
├── deb/{control,postinst,build-deb.sh}
├── rpm/{cava-bg.spec,build-rpm.sh}
└── aur/{PKGBUILD.in,render-pkgbuild.sh}
```

`deb/control` and `rpm/cava-bg.spec` are **templates**: `@VERSION@` is substituted at
build time. Do not write a literal version into them.

### Dependency lists are distro-specific

Package names are **not** portable. `wayland` exists on Fedora and Arch but **not**
on Debian, where it is `libwayland-client0` + `libwayland-egl1`. Never copy a
`Depends:`/`Requires:` line between the deb, rpm and AUR files — look each one up.

There is deliberately no `[package.metadata.deb]` / `[package.metadata.rpm]` in
`Cargo.toml`. `cargo deb` and `cargo generate-rpm` would read the dependency list
from there, creating a second source of truth that drifts from `packaging/`. That
drift is how a `Depends: wayland` line reached a published `.deb` and broke
`apt install`.

A linker-derived dependency list is not sufficient either: `readelf -d` on the
binary shows only FFmpeg and libc, because wgpu `dlopen`s `libEGL.so.1`,
`libwayland-client.so.0`, `libwayland-egl.so.1`, `libxkbcommon.so.0` and
`libxkbcommon-x11.so.0` at runtime. Those must be listed by hand.

The `deb` CI job builds the package inside a Debian container and installs it
with `apt-get install ./cava-bg.deb`, which fails on any unknown dependency name.

## Known landmines

Read this before touching the renderer.

* **The release profile deliberately does *not* set `panic = "abort"`.** The
  `panic::catch_unwind` keep-alive loop in `run_foreground` is real code that
  restarts the renderer after 2 s. Re-adding `abort` silently turns that into
  dead code and lets one bad GPU callback kill the daemon. The comment in
  `Cargo.toml` explains the trade-off — don't undo it without reading it first.
* **A `wgpu::Instance` is created per output and again on every surface rebuild.** Each
  one re-enumerates the whole system, and each output gets its own device, queue,
  shaders and pipelines. If you touch `ensure_output` / `rebuild_output_surface`,
  consider hoisting the `Instance` + `Device` + pipelines into shared, `Arc`'d state.
* **`mem::transmute` a `wgpu::Surface` to `'static`** (`wayland_renderer.rs`, in
  `rebuild_output_surface` and `ensure_output`). This is required by
  `create_surface_unsafe`, but combined with dropping and replacing surfaces it is a
  real lifetime hazard. Be careful not to introduce a path where the surface outlives
  what it points at.
* **Subprocess-per-frame polling.** `try_hyprctl_cursor` and `try_wlrctl_cursor` shell
  out to `hyprctl`/`wlrctl`; they back off from 16 ms to 100 ms while the pointer
  sits still (`AppState::CURSOR_POLL_IDLE`). The two wallpaper watchers share one
  `WATCH_STATE` and back off from 100 ms to 1 s via `poll_interval`. Before adding
  another poller, ask whether it can reuse those threads or the 5 s detection cache.
* **`generate_effect_frame` in `parallax_system.rs` reuses the previous frame's
  buffer** when nothing else holds a reference (checked via `Arc::try_unwrap`).
  Parallax frames are handed around as `Arc<VideoFrame>` precisely so that
  caching a frame and returning it to the consumer never copies the RGBA
  payload. If you add another consumer, keep it `Arc`-based.
* **`bar_geometry::vertices_per_bar` must be >= the real vertex count** of
  `build_bar` for every shape. Getting this wrong silently truncates the last
  bars. `bar_geometry::tests::vertices_per_bar_is_never_an_underestimate`
  guards this — extend it when you touch `build_bar`.
* **Duplicated logic.** The "output colours" decision and the `gradient_direction` →
  `f32` mapping are copy-pasted in four places in `wayland_renderer.rs`
  (`ensure_output`, `draw`, `apply_runtime_config`, `LayerShellHandler::configure`).
  A change needs four edits; consider extracting a helper when you touch one.
* **Dead code is masked by `#![allow(dead_code)]`** at the top of several modules
  (`app_config`, `parallax_system`, `video_decoder`, `wallpaper`, `layer_system`). Do
  not assume a field is wired up just because it exists and is editable in the GUI —
  verify with `grep` before documenting a config option as functional.

## Testing notes

* Tests are `#[cfg(test)] mod tests` in-module; there is no `tests/` directory.
* `cargo test` must pass **without a display, without audio and without a
  compositor**. Any new test that touches those needs to be structured so it doesn't.
* `wallpaper_detector::tests::from_ambxst_qml_detects_simulated_file` writes to
  `~/.config/ambxst/Wallpaper.qml`. Do not add more tests that mutate the real `$HOME`.
* Config tests belong in `app_config::tests`: assert on `toml::from_str::<Config>()`
  plus `resolve_for_output`, which is the pure, dependency-free part of the system.

## Commit and PR conventions

Conventional Commits, matching the existing history:

```
feat(renderer): add ring visualization smoothing
fix(config): stop panicking on malformed hex colours
perf(parallax): reuse the effect frame buffer
docs(agents): document the release flow
chore(deps): bump wgpu to 0.20
```

Scopes in use: `renderer`, `config`, `gui`, `parallax`, `supervisor`, `wallpaper`,
`video`, `xray`, `installer`, `packaging`, `release`, `deps`, `docs`, `chore`.
