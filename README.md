# <p align="center">cava-bg - Native CAVA Visualizer for Wayland</p>

<p align="center">
  <img src="multimedia/Example%20Xray2.gif">
</p>

**cava-bg** is a modern, lightweight, and highly customizable visualizer that turns any Wayland desktop into a dynamic audio experience. Designed for users who want seamless wallpaper integration without sacrificing performance, it features real-time color adaptation to your wallpaper and a unique "hidden image" reveal mode, making it the perfect choice for stylized desktop setups.

![Rust](https://img.shields.io/badge/Rust-1.70%2B-orange.svg)
![Wayland](https://img.shields.io/badge/Wayland-Native-green.svg)
![Arch Linux](https://img.shields.io/badge/Arch_Linux-AUR-blue.svg)

## Features

- **Native Wayland + wgpu rendering** – GPU‑accelerated visuals with low latency
- **Adaptive colors** – Extracts a gradient palette from your current wallpaper
- **Real‑time wallpaper monitoring** – Automatically updates colors when you change your wallpaper (supports ambxst, mpvpaper, waypaper, swaybg)
- **Full configuration** – TOML config file with options for bar count, gap, smoothing, framerate, and more
- **Static color fallback** – Use manually defined colors if dynamic extraction is disabled
- **Lightweight** – Spawns a single `cava` process and renders at your specified framerate
- **Multi‑output support** – Works on multiple monitors; can target a specific output
- **Kill command** – `cava-bg kill` stops any running instance

### Static Color Mode (Fallback or Manual)
If `dynamic_colors = false`, the visualizer uses user-defined colors from the `[colors]` section of the configuration file. Each color can be defined as a simple hex string (`"#rrggbb"`) or as an object containing both hex and alpha values.

### Flexible TOML Configuration
- **Default file path:** `~/.config/cava-bg/config.toml`
- **Key options include:**
    - `framerate`: Frames per second.
    - `amount`: Total number of vertical bars.
    - `gap`: Spacing between bars (as a fraction of bar width).
    - `bar_alpha`: Transparency level for the bars.
    - `corner_radius`: Corner rounding for the window (useful if the background isn't fully transparent).
    - `autosens` / `sensitivity`: CAVA sensitivity controls.
    - `preferred_output`: Target monitor name (e.g., "DP-1").
    - `background_color`: Background color for the overlay layer (fully transparent by default).

[Example Simple.webm](https://github.com/user-attachments/assets/4bb512ac-20aa-481f-be9f-94050b2d798a)

### Hidden Image Support
Displays a fixed image that is "revealed" by the bars as they move up and down.
- **Reveal Modes:** Currently supports `Reveal`.
- **Image Effects:** Apply filters to the revealed image such as `None`, `Grayscale`, `Invert`, `Sepia`, or predefined color palettes (`Catppuccin`, `Nord`, `Gruvbox`, `Solarized`).
- **Advanced Features:** You can use your current wallpaper as the hidden image or enable an automatic "x-ray" search in a specific directory to find stylized versions of your wallpaper for special visual effects.

<p align="center">
  <img src="multimedia/Example%20Xray1.gif" alt="Xray Effect Example">
</p>

## Installation

### Arch Linux (AUR)
####  Using yay
```bash
yay -S cava-bg
```
#### Using paru
```bash
paru -S cava-bg
```
### Debian/Ubuntu (DEB packages)

Download the .deb package from the releases section and install it with:

```bash
sudo dpkg -i cava-bg_<version>_amd64.deb
sudo apt-get install -f  # To fix dependencies if needed
```
### Fedora(RPM packages)

#### Download the .rpm package from the releases section and install it with:

```bash
sudo rpm -i cava-bg-<version>.rpm
```
#### or using dnf
```bash
sudo dnf install cava-bg-<version>.rpm
```

### From Source

1. **Install Dependencies:**
```bash
# Arch Linux
sudo pacman -S cava rustup wayland wayland-protocols libxkbcommon mesa libglvnd
```
```bash
# Debian/Ubuntu
sudo apt install cava rustc cargo libwayland-dev wayland-protocols libxkbcommon-dev libgl1-mesa-dev libglvnd-dev
```
```bash
# Fedora
sudo dnf install cava rust cargo wayland-devel wayland-protocols-devel libxkbcommon-devel mesa-libGL-devel libglvnd-devel
```

2. **Build and Install:**
```bash
git clone https://github.com/leriart/cava-bg.git
cd cava-bg
cargo build --release
sudo cp target/release/cava-bg /usr/local/bin/
```

3. **Test Installation:**
```bash
# Run cava-bg
cava-bg
```

## Usage
```bash
# Start with default config (~/.config/cava-bg/config.toml)
cava-bg

# Use a custom config file
cava-bg --config /path/to/config.toml

# Stop the running instance
cava-bg kill
```
## How It Works

- **Wallpaper detection** – The program locates your current wallpaper by checking common tools: ambxst, mpvpaper, waypaper, swaybg.
- **Color extraction** – Using color_thief, it extracts a palette of dominant colors, sorts them by luminance, and applies temporal smoothing to avoid abrupt changes.
- **CAVA integration** – Spawns a cava process with a raw 16‑bit output, reads the audio data, and normalises the values.
- **Wayland layer** – Creates a wlr_layer_shell surface anchored to the whole screen, with a transparent background.
- **wgpu rendering** – A full‑screen quad is drawn for each bar. A fragment shader interpolates the gradient vertically. The uniform buffer is updated whenever the wallpaper colors change.
- **Wallpaper monitoring** – Every 2 seconds the wallpaper path is rechecked; if it changed, a new palette is generated and sent to the render thread.

## Wallpaper Managers
- swww
- hyprpaper
- wpaperd
- swaybg
- mpvpaper
- awww

## Hyprland Shells/Dotfiles
- ambxst

## Desktop Environments
- GNOME (Wayland)
- KDE Plasma (Wayland)
- Cinnamon
- Budgie
- XFCE
- MATE
- LXQt
- Deepin
- Enlightenment

## Acknowledgments

### Projects & Libraries
- **wallpaper-cava** - Inspiration and audio processing approach
- **CAVA** - Audio visualization engine
- **Smithay Client Toolkit** - Wayland client library
- **wgpu** - Modern graphics API for Rust
