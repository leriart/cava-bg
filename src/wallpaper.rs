use anyhow::{Context, Result};
use color_thief::{get_palette, ColorFormat};
use image::{self, GenericImageView};
use log;
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.7;

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    /// Returns the path of the current wallpaper if a known manager is active.
    pub fn find_wallpaper() -> Option<PathBuf> {
        // 1. mpvpaper (very common for GIFs/videos)
        if let Some(path) = Self::from_mpvpaper() {
            log::info!("Detected wallpaper via mpvpaper: {:?}", path);
            return Some(path);
        }

        // 2. swww / awww (daemons with query command)
        if let Some(path) = Self::from_swww_like("awww") {
            log::info!("Detected wallpaper via awww: {:?}", path);
            return Some(path);
        }
        if let Some(path) = Self::from_swww_like("swww") {
            log::info!("Detected wallpaper via swww: {:?}", path);
            return Some(path);
        }

        // 3. swaybg (classic Wayland wallpaper setter)
        if let Some(path) = Self::from_swaybg() {
            log::info!("Detected wallpaper via swaybg: {:?}", path);
            return Some(path);
        }

        // 4. hyprpaper (via hyprctl)
        if let Some(path) = Self::from_hyprctl_hyprpaper() {
            log::info!("Detected wallpaper via hyprctl hyprpaper: {:?}", path);
            return Some(path);
        }

        // 5. hyprpaper (via config file)
        if let Some(path) = Self::from_hyprpaper_conf() {
            log::info!("Detected wallpaper via hyprpaper.conf: {:?}", path);
            return Some(path);
        }

        // 6. wpaperd (state file)
        if let Some(path) = Self::from_wpaperd() {
            log::info!("Detected wallpaper via wpaperd: {:?}", path);
            return Some(path);
        }

        // 7. wbg
        if let Some(path) = Self::from_wbg() {
            log::info!("Detected wallpaper via wbg: {:?}", path);
            return Some(path);
        }

        // 8. waypaper (frontend config)
        if let Some(path) = Self::from_waypaper() {
            log::info!("Detected wallpaper via waypaper config: {:?}", path);
            return Some(path);
        }

        log::warn!("No active wallpaper manager detected");
        None
    }

    // --- Detection helpers ---

    fn from_hyprctl_hyprpaper() -> Option<PathBuf> {
        let output = Command::new("hyprctl")
            .args(["hyprpaper", "listloaded"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.starts_with("wallpaper ") {
                let path_str = line.trim_start_matches("wallpaper ").trim();
                let path = PathBuf::from(path_str);
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_swww_like(cmd: &str) -> Option<PathBuf> {
        let output = Command::new(cmd).arg("query").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some((_monitor, path_str)) = line.split_once(':') {
                let path = PathBuf::from(path_str.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_swaybg() -> Option<PathBuf> {
        let output = Command::new("pgrep").arg("-a").arg("swaybg").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("swaybg") && line.contains(" -i ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pos) = parts.iter().position(|&s| s == "-i") {
                    if let Some(path_str) = parts.get(pos + 1) {
                        let path = PathBuf::from(path_str);
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    fn from_hyprpaper_conf() -> Option<PathBuf> {
        let conf = dirs::config_dir()?.join("hypr/hyprpaper.conf");
        if !conf.exists() {
            return None;
        }
        let content = std::fs::read_to_string(conf).ok()?;
        for line in content.lines() {
            if line.starts_with("wallpaper") || line.starts_with("preload") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() >= 2 {
                    let right = parts[1].trim();
                    if let Some((_, path_str)) = right.split_once(',') {
                        let path = PathBuf::from(path_str);
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    fn from_mpvpaper() -> Option<PathBuf> {
        // Use pgrep -a to get full command line of mpvpaper
        let output = Command::new("pgrep").arg("-a").arg("mpvpaper").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("mpvpaper") {
                // The last argument that looks like a file path is likely the wallpaper.
                let parts: Vec<&str> = line.split_whitespace().collect();
                for part in parts.iter().rev() {
                    let path = PathBuf::from(part);
                    if path.exists() {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            let ext_lower = ext.to_lowercase();
                            if matches!(ext_lower.as_str(),
                                "jpg" | "jpeg" | "png" | "bmp" | "webp" | "gif" |
                                "mp4" | "mkv" | "webm" | "avi" | "mov")
                            {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn from_wpaperd() -> Option<PathBuf> {
        let state_file = dirs::runtime_dir()?.join("wpaperd.state");
        if state_file.exists() {
            if let Ok(content) = std::fs::read_to_string(state_file) {
                let path = PathBuf::from(content.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_wbg() -> Option<PathBuf> {
        let output = Command::new("pgrep").arg("-a").arg("wbg").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let path = PathBuf::from(parts[1]);
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_waypaper() -> Option<PathBuf> {
        let conf = dirs::config_dir()?.join("waypaper/config.ini");
        if !conf.exists() {
            return None;
        }
        let content = std::fs::read_to_string(conf).ok()?;
        for line in content.lines() {
            if line.starts_with("wallpaper") {
                if let Some((_, path_str)) = line.split_once('=') {
                    let path = PathBuf::from(path_str.trim().trim_matches('"'));
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    // --- Image loading ---

    fn load_image_from_path(path: &PathBuf) -> Result<image::DynamicImage> {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "avi" | "mov") {
            let temp_frame = std::env::temp_dir().join("cava_bg_temp_frame.png");
            let status = Command::new("ffmpeg")
                .args(["-i", path.to_str().unwrap(), "-vframes", "1", "-q:v", "2", temp_frame.to_str().unwrap(), "-y"])
                .status();
            if let Ok(status) = status {
                if status.success() {
                    let img = image::open(&temp_frame)
                        .context("Failed to open video frame")?;
                    let _ = std::fs::remove_file(temp_frame);
                    return Ok(img);
                }
            }
            anyhow::bail!("Could not extract frame from video");
        }

        image::open(path).context("Failed to open image")
    }

    pub fn default_colors(num_colors: usize) -> Vec<[f32; 4]> {
        let catppuccin = [
            [0.580, 0.886, 0.835, 1.0],
            [0.537, 0.863, 0.922, 1.0],
            [0.455, 0.780, 0.925, 1.0],
            [0.537, 0.706, 0.980, 1.0],
            [0.796, 0.651, 0.969, 1.0],
            [0.961, 0.761, 0.906, 1.0],
            [0.922, 0.627, 0.675, 1.0],
            [0.953, 0.545, 0.659, 1.0],
        ];
        if num_colors <= catppuccin.len() {
            catppuccin[0..num_colors].to_vec()
        } else {
            let mut colors = Vec::new();
            for i in 0..num_colors {
                colors.push(catppuccin[i % catppuccin.len()]);
            }
            colors
        }
    }

    /// Generate a color palette from the current wallpaper using color-thief (Median Cut).
    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper() {
            Some(path) => path,
            None => {
                // Already logged, just return defaults.
                return Ok(Self::default_colors(num_colors));
            }
        };

        log::info!("Analyzing wallpaper: {:?}", wallpaper_path);

        let img = match Self::load_image_from_path(&wallpaper_path) {
            Ok(img) => img,
            Err(e) => {
                log::warn!("Could not load wallpaper image: {}, using default colors", e);
                return Ok(Self::default_colors(num_colors));
            }
        };

        let (width, height) = img.dimensions();
        log::debug!("Wallpaper dimensions: {}x{}", width, height);

        let rgb_img = img.to_rgb8();
        let pixels = rgb_img.as_raw();

        let palette = get_palette(pixels, ColorFormat::Rgb, 10, num_colors as u8)
            .context("Failed to extract color palette")?;

        let mut new_colors: Vec<[f32; 4]> = palette
            .iter()
            .map(|c| [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, 1.0])
            .collect();

        // Sort by luminance for a smoother gradient
        new_colors.sort_by(|a, b| {
            let lum_a = 0.299 * a[0] + 0.587 * a[1] + 0.114 * a[2];
            let lum_b = 0.299 * b[0] + 0.587 * b[1] + 0.114 * b[2];
            lum_a.partial_cmp(&lum_b).unwrap()
        });

        let mut prev_guard = PREVIOUS_COLORS.lock().unwrap();
        if !prev_guard.is_empty() && prev_guard.len() == new_colors.len() {
            for i in 0..new_colors.len() {
                for c in 0..4 {
                    new_colors[i][c] = COLOR_SMOOTHING_FACTOR * new_colors[i][c]
                        + (1.0 - COLOR_SMOOTHING_FACTOR) * prev_guard[i][c];
                }
            }
        }
        *prev_guard = new_colors.clone();

        log::info!("New gradient colors:");
        for (i, color) in new_colors.iter().enumerate() {
            log::info!("  Color {}: #{:02x}{:02x}{:02x} (alpha: {:.2})",
                i+1,
                (color[0]*255.0) as u8,
                (color[1]*255.0) as u8,
                (color[2]*255.0) as u8,
                color[3]);
        }

        Ok(new_colors)
    }
}