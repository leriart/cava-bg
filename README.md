# cava-bg - Native CAVA Visualizer for Hyprland

A native implementation of wallpaper-cava optimized for Hyprland, displaying CAVA audio visualizations as a layer over your wallpaper with adaptive color detection.

![cava-bg Demo](https://img.shields.io/badge/demo-coming_soon-blue)
![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)
![License](https://img.shields.io/badge/license-MIT-green.svg)
![Platform](https://img.shields.io/badge/platform-Linux%20Wayland-lightgrey)

## Features

- **Adaptive Gradient Colors** - Automatically extracts and generates gradient colors from your wallpaper
- **Wallpaper Change Detection** - Detects wallpaper changes and updates colors in real-time
- **Native Wayland Integration** - Uses wlr-layer-shell for optimal performance on Hyprland
- **Real-time Audio Visualization** - Connects directly to cava for audio processing
- **Hardware Accelerated** - OpenGL 4.6 rendering for smooth visuals
- **Automatic Configuration** - Self-adjusting based on your wallpaper and preferences

## Quick Install

### AUR (Arch Linux)

```bash
# Using paru
paru -S cava-bg

# Using yay
yay -S cava-bg
```

### Binary Release

```bash
# Download latest release
VERSION="0.1.0"
ARCH="x86_64-unknown-linux-gnu"
wget https://github.com/leriart/cava-bg/releases/download/v${VERSION}/cava-bg-v${VERSION}-${ARCH}.tar.gz

# Extract and install
sudo tar -xzf cava-bg-v${VERSION}-${ARCH}.tar.gz -C /usr/local/bin/

# Run (cava will be installed automatically if needed)
cava-bg
```

### From Source
```bash
git clone https://github.com/leriart/cava-bg.git
cd cava-bg
cargo build --release
sudo cp target/release/cava-bg /usr/local/bin/
```

## Installation

### Method 1: Binary Release (Easiest)

Download the pre-built binary from the [Releases](https://github.com/leriart/cava-bg/releases) page:

```bash
# Example for v0.1.0
curl -L https://github.com/leriart/cava-bg/releases/download/v0.1.0/cava-bg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz | tar -xz
sudo mv cava-bg /usr/local/bin/
```

### Method 2: From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/cavabg.git
cd cavabg

# Build in release mode
cargo build --release

# Install system-wide
sudo cp target/release/cavabg /usr/local/bin/
```

### Method 3: Using install.sh

```bash
chmod +x install.sh
./install.sh
```

### Dependencies

**Required:**
- `cava` - Audio visualizer
- Wayland compositor with wlr-layer-shell support (Hyprland, Sway, etc.)
- OpenGL 4.6 capable GPU

**Install on Arch Linux:**
```bash
sudo pacman -S cava base-devel pkg-config wayland-protocols libxkbcommon
```

**Install on Ubuntu/Debian:**
```bash
sudo apt install cava build-essential pkg-config libwayland-dev libegl-dev mesa-common-dev libxkbcommon-dev wayland-protocols
```

## Configuration

After installation, set up your configuration:

```bash
# Create config directory
mkdir -p ~/.config/cavabg

# Copy default config
cp config.toml ~/.config/cavabg/

# Edit to your liking
nano ~/.config/cavabg/config.toml
```

Or use the built-in default configuration if you don't create one.

### Configuration Options

| Section | Option | Type | Default | Description |
|---------|--------|------|---------|-------------|
| `[general]` | `framerate` | integer | 60 | FPS for visualization |
| | `background_color` | color | `{hex: "#000000", alpha: 0.0}` | Background with transparency |
| | `preferred_output` | string | (none) | Target monitor name |
| `[bars]` | `amount` | integer | 76 | Number of bars |
| | `gap` | float | 0.1 | Gap between bars (0.0-1.0) |
| `[colors]` | `gradient_color_*` | color | Catppuccin gradient | Gradient colors (any number) |
| `[smoothing]` | `noise_reduction` | float | 0.77 | CAVA noise reduction (0.0-1.0) |

**Color formats:**
- `"#RRGGBB"` - Hex color (alpha = 1.0)
- `{hex: "#RRGGBB", alpha: 0.5}` - Hex color with alpha

## Usage

### Basic Usage

```bash
# Run with default config
cavabg

# Run with specific config
cavabg --config ~/.config/cavabg/my-config.toml

# Show help
cavabg --help
```

### Hyprland Integration

Add to `~/.config/hypr/hyprland.conf`:

```hyprlang
# Start on login
exec-once = cavabg

# With custom config
exec-once = cavabg --config ~/.config/cavabg/config.toml

# Delay start (wait 2 seconds)
exec-once = sleep 2 && cavabg
```

### Monitor Targeting

Target specific monitor:
```bash
# Get monitor names
hyprctl monitors all

# Set in config.toml
[general]
preferred_output = "DP-1"
```

## Building from Source

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Install dependencies (Arch example)
sudo pacman -S cava base-devel pkg-config wayland-protocols
```

### Build

```bash
# Clone
git clone https://github.com/yourusername/cavabg.git
cd cavabg

# Debug build
cargo build

# Release build (recommended)
cargo build --release

# Run tests
cargo test
```

### Create Release

```bash
# Build release package
./build-release.sh

# Outputs:
# - dist/cavabg (binary)
# - cavabg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
# - cavabg-v0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
```

## Troubleshooting

### Common Issues

| Issue | Solution |
|-------|----------|
| **"cava not found"** | `sudo pacman -S cava` or `sudo apt install cava` |
| **"wl_compositor not available"** | Make sure you're running Wayland (`echo $XDG_SESSION_TYPE`) |
| **Black screen** | Check audio input to cava, verify config |
| **Low FPS** | Reduce `framerate` or `bars.amount` in config |
| **Permission denied** | `chmod +x /path/to/cavabg` or use `sudo` |

### Debug Mode

```bash
# Enable verbose logging
RUST_LOG=debug cavabg

# Or with custom config
RUST_LOG=info cavabg --config ~/.config/cavabg/config.toml
```

### Checking Dependencies

```bash
# Check cava
cava --version

# Check Wayland
echo $XDG_SESSION_TYPE

# Check OpenGL
glxinfo | grep "OpenGL version"
```

## Project Structure

```
cavabg/
├── src/                    # Source code
│   ├── main.rs            # Application entry point
│   ├── config.rs          # Configuration parsing
│   ├── shader.rs          # OpenGL shader management
│   └── shaders/           # GLSL shaders
├── Cargo.toml            # Rust dependencies
├── build.rs              # OpenGL bindings generation
├── config.toml           # Example configuration
├── build-release.sh      # Release builder
├── install.sh            # Installation script
├── README.md             # This file
├── INSTALL.md            # Detailed installation guide
└── DEVELOPMENT.md        # Development guide
```

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Commit changes: `git commit -m 'Add amazing feature'`
4. Push: `git push origin feature/amazing-feature`
5. Open a Pull Request

See [DEVELOPMENT.md](DEVELOPMENT.md) for development setup.

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Acknowledgments

- Based on [wallpaper-cava](https://github.com/rs-pro0/wallpaper-cava) by rs-pro0
- Uses [Smithay client toolkit](https://github.com/Smithay/client-toolkit)
- Inspired by [cava](https://github.com/karlstav/cava) community
- Catppuccin color scheme by [catppuccin](https://github.com/catppuccin)

---

**If you find this useful, please star the repository!**
