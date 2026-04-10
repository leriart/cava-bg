# Changelog

All notable changes to cava-bg will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] - Development Version

### Added
- **Efficient audio processing** inspired by wallpaper-cava (raw 16-bit format)
- **New cava_manager module** for optimized cava process management
- **Advanced renderer structure** with Wayland detection
- **Improved user feedback** with detailed audio monitoring
- **ASCII visualization** in terminal mode
- **Dynamic versioning** in PKGBUILD (git-based)
- **Enhanced configuration** with raw audio output support

### Changed
- **PKGBUILD now uses git source** (always latest main branch)
- **Audio processing completely rewritten** for efficiency
- **User interface significantly improved** with real-time feedback
- **Project structure optimized** for future graphical rendering
- **Dependencies updated** for better compatibility

### Fixed
- **CLI argument conflicts** resolved (flag collisions)
- **Compiler warnings** cleaned up
- **Unused code removed** for cleaner codebase
- **Build process streamlined**

### Technical Improvements
- **Architecture prepared** for full Wayland/OpenGL implementation
- **Shader system structured** for future gradient rendering
- **Audio data pipeline optimized** (wallpaper-cava inspired)
- **Error handling enhanced** with better fallback mechanisms
- **Code organization improved** with modular design

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