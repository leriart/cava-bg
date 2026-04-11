# cava-bg - Native CAVA Visualizer for Hyprland

A native implementation of wallpaper-cava optimized for Hyprland, displaying CAVA audio visualizations as a layer over your wallpaper with adaptive color detection.

![Rust](https://img.shields.io/badge/Rust-1.70%2B-orange.svg)
![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)
![Wayland](https://img.shields.io/badge/Wayland-Native-green.svg)
![Arch Linux](https://img.shields.io/badge/Arch_Linux-AUR-blue.svg)

## Features

### Currently Implemented:
- **Adaptive Color Detection**: Automatically extracts colors from your current wallpaper
- **Efficient Audio Processing**: Raw 16-bit CAVA output (inspired by wallpaper-cava)
- **Terminal Visualization**: Real-time ASCII audio bars with level indicators
- **Configuration System**: TOML-based config with auto-reload
- **Wallpaper Monitoring**: Detects wallpaper changes and updates colors
- **Wayland Detection**: Auto-detects Wayland sessions and compositors
- **Modular Architecture**: Ready for full graphical rendering implementation

### Structure Ready for Implementation:
- **Wayland/OpenGL Rendering**: Full GPU-accelerated visualization pipeline
- **GLSL Shaders**: Advanced gradient and effects system (shaders prepared)
- **Multi-monitor Support**: Architecture prepared for multiple displays
- **Advanced Effects**: Pulse, glow, and smooth animations system
- **Performance Optimization**: SSBO data transfer and instanced rendering ready

## Installation

### Quick Install (Arch Linux AUR)
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

### Basic Usage
```bash
# Start with default settings
cava-bg

# Test configuration and color extraction
RUST_LOG=info cava-bg
```

### Command Line Options
```bash
cava-bg --help
```

**Available Options:**
- `--config <PATH>`: Use custom config file
- `--test-config`: Test color extraction and exit
- `--version`: Show version information
- `--list-monitors` (`-m`): List available monitors
- `--wayland`: Force Wayland rendering mode (if available)

## Configuration

### Basic Configuration
Create `~/.config/cava-bg/config.toml`:

```toml
[general]
framerate = 60
auto_colors = true
wallpaper_check_interval = 5

[bars]
amount = 76
width = 1.0
gap = 0.1
roundness = 0.2

# Colors are auto-generated from wallpaper
# Manual override available in [colors] section
```

### Configuration Locations
cava-bg looks for configuration in this order:
1. Command line: `--config /path/to/config.toml`
2. `~/.config/cava-bg/config.toml`
3. `/etc/cava-bg/config.toml`
4. Built-in defaults

## How It Works

### Current Implementation Pipeline:
```
1. Wallpaper Detection -> Find current wallpaper path
2. Color Extraction -> k-means clustering for dominant colors
3. Gradient Generation -> Create smooth color gradients
4. Audio Processing -> CAVA raw 16-bit binary output
5. Data Normalization -> Convert to 0.0-1.0 range
6. Terminal Rendering -> ASCII visualization with real-time feedback
7. Wayland Detection -> Check for graphical rendering capability
```

### Future Graphics Pipeline (Structure Ready):
```
1. Wayland Connection -> Connect to compositor
2. Surface Creation -> wlr-layer-shell transparent window
3. EGL/OpenGL Context -> GPU acceleration setup
4. Shader Compilation -> GLSL vertex/fragment shaders
5. Buffer Setup -> VBO/VAO/EBO + SSBO for audio data
6. Main Render Loop -> 60 FPS frame rendering
7. Audio Data Transfer -> SSBO updates each frame
8. Effects Application -> Pulse, glow, gradient blending
```

## Project Structure

```
cava-bg/
├── src/
│   ├── main.rs              # Application entry point
│   ├── config.rs           # TOML configuration parsing
│   ├── wallpaper.rs        # Wallpaper analysis & color extraction
│   ├── renderer.rs         # Modular rendering system
│   ├── cava_manager.rs     # Efficient CAVA process management
│   ├── cli.rs             # Command line interface (clap)
│   ├── wayland_basic.rs   # Basic Wayland implementation
│   ├── wayland_simple.rs  # Simple renderer structure
│   └── wayland_renderer.rs # Full Wayland renderer (future)
├── shaders/               # GLSL shaders
│   ├── vertex.glsl        # Vertex shader (ready)
│   └── fragment.glsl      # Fragment shader with effects (ready)
└── Cargo.toml            # Rust dependencies
```

## Development

### Building from Source
```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Check code
cargo check
```

## Contributing

We welcome contributions! Here's how to get started:

### Development Setup
```bash
# 1. Fork and clone
git clone https://github.com/your-username/cava-bg.git
cd cava-bg

# 2. Install dependencies
sudo pacman -S cava rustup wayland wayland-protocols libxkbcommon mesa libglvnd

# 3. Build and test
cargo build
cargo test

# 4. Create feature branch
git checkout -b feature/your-feature
```

### Contribution Guidelines
1. **Code Style**: Follow Rust conventions, run `cargo fmt` before committing
2. **Testing**: Add tests for new functionality
3. **Documentation**: Update relevant documentation
4. **Commits**: Use meaningful commit messages
5. **PRs**: Reference related issues, describe changes clearly

## Troubleshooting

### Common Issues & Solutions

#### 1. No Audio Detected
```bash
# Check CAVA installation
cava --version

# Test CAVA directly
cava -p /dev/null
```

#### 2. Wayland Not Detected
```bash
# Check session type
echo $XDG_SESSION_TYPE
echo $WAYLAND_DISPLAY

# Install Wayland compositor (Arch Linux)
sudo pacman -S hyprland
```

#### 3. Debug Mode
```bash
# Full debug output
RUST_LOG=debug cava-bg

# Specific module debugging
RUST_LOG=cava_bg=debug,cava_bg::renderer=info cava-bg
```

## Acknowledgments

### Projects & Libraries
- **wallpaper-cava** - Inspiration and audio processing approach
- **CAVA** - Audio visualization engine
- **Smithay Client Toolkit** - Wayland client library
- **wlroots** - Wayland compositor library

### Rust Ecosystem
- **anyhow** - Error handling
- **clap** - Command line parsing
- **toml** - Configuration parsing
- **log/env_logger** - Logging system
- **image** - Image processing for color extraction


