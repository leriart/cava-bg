# cava-bg - Native CAVA Visualizer for Wayland

A high-performance audio visualizer for Wayland compositors (Hyprland, Sway, River, etc.) that displays real-time CAVA audio bars as a transparent overlay over your wallpaper, with automatic color extraction and dynamic updates when your wallpaper changes.

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

## Installation

### Arch Linux (AUR)

```bash
# Using yay
yay -S cava-bg

# Using paru
paru -S cava-bg
```

### From Source

1. **Install Dependencies:**
```bash
# Arch Linux
sudo pacman -S cava rustup wayland wayland-protocols libxkbcommon mesa libglvnd
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

### Supported Wallpaper Tools

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
