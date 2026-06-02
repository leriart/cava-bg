%global __provides_exclude_from ^/usr/bin/cava-bg$
%global __requires_exclude ^.*$

Name:           cava-bg
Version:        0.2.4
Release:        1%{?dist}
Summary:        Audio visualizer for Wayland — background layer

License:        MIT
URL:            https://github.com/leriart/cava-bg
Source0:        https://github.com/leriart/cava-bg/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  pkg-config
BuildRequires:  wayland-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  mesa-libEGL-devel
BuildRequires:  mesa-libGL-devel
BuildRequires:  dbus-devel
BuildRequires:  ffmpeg-devel

Requires:       cava
Requires:       wayland
Requires:       ffmpeg
Requires:       libxkbcommon
Requires:       mesa-libEGL
Requires:       mesa-libGL
Requires:       dbus-libs

%description
cava-bg draws real-time audio visualization bars over the wallpaper
in Wayland compositors. It acts as a wlr-layer-shell background layer,
supporting Hyprland, Sway, and other wlroots-based environments.

Features:
 * Real-time audio bars via cava
 * Adaptive gradient colors from wallpaper
 * Automatic wallpaper change detection
 * GUI configurator (--config)
 * Wallpaper X-ray mode
 * Parallax effects
 * Multiple bar shapes and display modes

%prep
%autosetup -n cava-bg-%{version}

%build
export RUSTUP_TOOLCHAIN=stable
%{__cargo} build --release --locked

%install
# Binary
install -Dm755 target/release/cava-bg %{buildroot}%{_bindir}/cava-bg

# Config
install -Dm644 config.toml %{buildroot}%{_docdir}/%{name}/config.toml

# Docs
install -Dm644 README.md %{buildroot}%{_docdir}/%{name}/README.md
install -Dm644 LICENSE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE

# Desktop entry
mkdir -p %{buildroot}%{_datadir}/applications
cat > %{buildroot}%{_datadir}/applications/%{name}.desktop << 'EOF'
[Desktop Entry]
Name=cava-bg Config
Comment=Configure cava-bg audio visualizer
Exec=cava-bg --config
Icon=audio-card
Terminal=false
Type=Application
Categories=AudioVideo;Audio;
EOF

%files
%license LICENSE
%doc README.md
%{_bindir}/cava-bg
%{_datadir}/applications/cava-bg.desktop
%{_docdir}/%{name}/

%changelog
* Tue Jun 02 2026 Leriart - 0.2.4-1
- Initial RPM packaging
