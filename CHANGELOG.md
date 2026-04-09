# Changelog

All notable changes to cava-bg will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial project structure
- Basic audio visualization with cava integration
- Wayland wlr-layer-shell support
- OpenGL 4.6 rendering with gradient colors
- Configuration file support (TOML)
- Multi-monitor targeting
- Installation scripts
- GitHub Actions CI/CD

### Changed
- N/A

### Fixed
- N/A

## [0.1.0] - 2024-04-09

### Added
- Initial release
- Basic functionality from wallpaper-cava
- Improved error handling with anyhow
- Modular code structure
- Comprehensive documentation
- Release build system

### Features
- Real-time audio visualization
- Customizable gradient colors
- Adjustable bar count and gaps
- Transparent background support
- Hyprland integration
- Multi-monitor support
- Configurable smoothing parameters

### Technical
- Rust-based implementation
- OpenGL 4.6 with EGL
- Wayland native (wlr-layer-shell)
- TOML configuration
- Logging with env_logger