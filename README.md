# Cavabg - Native CAVA Visualizer for Hyprland

A native implementation of wallpaper-cava optimized for Hyprland, displaying CAVA audio visualizations as a layer over your wallpaper.

## Features

- **Native Hyprland integration**: Uses wlr-layer-shell protocol directly
- **Hardware-accelerated rendering**: OpenGL 4.6 with EGL
- **Customizable visuals**: Gradient colors, bar count, gaps, and more
- **Real-time audio visualization**: Connects directly to cava for audio processing
- **Transparent background**: Can be configured with alpha transparency
- **Multi-monitor support**: Can target specific outputs

## Prerequisites

- **Rust toolchain** (latest stable)
- **cava** audio visualizer (`pacman -S cava` on Arch)
- **Hyprland** or any Wayland compositor supporting wlr-layer-shell
- **OpenGL 4.6** capable GPU with EGL support

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/cavabg.git
cd cavabg

# Build in release mode
cargo build --release

# The binary will be at target/release/cavabg
```

### Dependencies

On Arch Linux:
```bash
sudo pacman -S cava base-devel
```

## Configuration

Copy `config.toml` to `~/.config/cavabg/config.toml` and customize:

```bash
mkdir -p ~/.config/cavabg
cp config.toml ~/.config/cavabg/
```

### Configuration Options

- **general.framerate**: FPS for the visualization (default: 60)
- **general.background_color**: Background color with alpha (default: transparent black)
- **bars.amount**: Number of bars (default: 76)
- **bars.gap**: Gap between bars as percentage of bar width (default: 0.1)
- **colors**: Gradient colors (supports any number of colors)
- **smoothing**: CAVA smoothing parameters

## Usage

### Basic Usage

```bash
# Run with default configuration
cavabg

# Run with specific config file
cavabg --config /path/to/config.toml
```

### Hyprland Integration

Add to your Hyprland configuration (`~/.config/hypr/hyprland.conf`):

```hyprlang
# Start cavabg on launch
exec-once = cavabg

# Or with specific config
exec-once = cavabg --config ~/.config/cavabg/my-config.toml
```

### Monitor Targeting

To target a specific monitor, set the monitor name in config:

```toml
[general]
preferred_output = "DP-1"
```

Get monitor names with:
```bash
hyprctl monitors all
```

## Building from Source

```bash
# Clone and build
git clone https://github.com/yourusername/cavabg.git
cd cavabg
cargo build --release

# Install system-wide (optional)
sudo cp target/release/cavabg /usr/local/bin/
```

## Troubleshooting

### Common Issues

1. **"cava not found"**: Install cava: `sudo pacman -S cava`
2. **"wl_compositor not available"**: Make sure you're running under Wayland
3. **Black screen**: Check that cava is receiving audio input
4. **Low performance**: Reduce bar count or framerate in config

### Debug Mode

Run with RUST_LOG environment variable for debug output:

```bash
RUST_LOG=debug cavabg
```

## License

MIT

## Acknowledgments

- Based on [wallpaper-cava](https://github.com/rs-pro0/wallpaper-cava) by rs-pro0
- Uses [Smithay client toolkit](https://github.com/Smithay/client-toolkit)
- Inspired by the cava community