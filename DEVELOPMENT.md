# Development Setup

## Prerequisites Installation

### 1. Install Rust

```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add to PATH (or restart shell)
source "$HOME/.cargo/env"

# Verify installation
rustc --version
cargo --version
```

### 2. Install System Dependencies

**Arch Linux:**
```bash
sudo pacman -S cava base-devel pkg-config wayland-protocols wayland
```

**Ubuntu/Debian:**
```bash
sudo apt install cava build-essential pkg-config libwayland-dev libegl-dev mesa-common-dev
```

### 3. Build and Run

```bash
# Build in debug mode
cargo build

# Build in release mode (recommended)
cargo build --release

# Run with default config
cargo run --release

# Run with specific config
cargo run --release -- --config config.toml
```

## Project Structure

```
cavabg/
├── src/
│   ├── main.rs          # Main application logic
│   ├── config.rs        # Configuration parsing
│   ├── shader.rs        # OpenGL shader management
│   └── shaders/
│       ├── vertex_shader.glsl
│       └── fragment_shader.glsl
├── Cargo.toml          # Rust dependencies
├── build.rs            # OpenGL bindings generation
├── config.toml         # Example configuration
├── README.md           # Documentation
├── install.sh          # Installation script
└── DEVELOPMENT.md      # This file
```

## Testing

### Manual Testing

1. **Audio Input Test:**
   ```bash
   # Test cava separately
   cava -p config.toml
   ```

2. **Wayland Environment:**
   ```bash
   # Check Wayland session
   echo $XDG_SESSION_TYPE
   # Should output "wayland"
   ```

3. **OpenGL Support:**
   ```bash
   # Check OpenGL version
   glxinfo | grep "OpenGL version"
   ```

### Debug Logging

Enable debug output:
```bash
RUST_LOG=debug cargo run --release
```

## Common Issues

### 1. "cava not found"
```bash
sudo pacman -S cava  # Arch
sudo apt install cava # Debian/Ubuntu
```

### 2. Wayland Protocol Errors
Ensure you're running under Wayland:
```bash
# Check session type
echo $XDG_SESSION_TYPE

# If using X11, switch to Wayland
# For Hyprland, make sure you're logging into a Wayland session
```

### 3. OpenGL/EGL Errors
Install Mesa drivers:
```bash
sudo pacman -S mesa  # Arch
sudo apt install mesa-utils # Debian/Ubuntu
```

### 4. Build Dependencies Missing
Install development packages:
```bash
sudo pacman -S base-devel pkg-config wayland-protocols
sudo apt install build-essential pkg-config libwayland-dev
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make changes
4. Test thoroughly
5. Submit a pull request

## License

MIT