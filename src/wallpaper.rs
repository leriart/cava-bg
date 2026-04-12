use anyhow::{Context, Result};
use color_thief::{get_palette, ColorFormat};
use image::{self, GenericImageView};
use log;
use once_cell::sync::Lazy;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.7;

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    pub fn find_wallpaper() -> Option<PathBuf> {
        // 1. ambxst
        if let Some(path) = Self::from_ambxst() {
            log::info!("Detected wallpaper via ambxst: {:?}", path);
            return Some(path);
        }

        // 2. mpvpaper
        if let Some(path) = Self::from_mpvpaper() {
            log::info!("Detected wallpaper via mpvpaper: {:?}", path);
            return Some(path);
        }

        // 3. Waypaper
        if let Some(path) = Self::from_waypaper() {
            log::info!("Detected wallpaper via Waypaper: {:?}", path);
            return Some(path);
        }

        // 4. swaybg
        if let Some(path) = Self::from_swaybg() {
            log::info!("Detected wallpaper via swaybg: {:?}", path);
            return Some(path);
        }

        None
    }

    fn from_ambxst() -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let cache_path = home.join(".cache/ambxst/wallpapers.json");
        log::debug!("Looking for ambxst config at: {:?}", cache_path);
        if !cache_path.exists() {
            log::debug!("ambxst config file not found");
            return None;
        }
        let content = fs::read_to_string(cache_path).ok()?;
        log::debug!("ambxst config content: {}", content);

        let tag = "currentWall";
        if let Some(start) = content.find(tag) {
            let after_tag = &content[start + tag.len()..];
            if let Some(colon_pos) = after_tag.find(':') {
                let after_colon = &after_tag[colon_pos + 1..];
                if let Some(quote_start) = after_colon.find('"') {
                    let after_quote = &after_colon[quote_start + 1..];
                    if let Some(quote_end) = after_quote.find('"') {
                        let path_str = &after_quote[..quote_end];
                        let path = PathBuf::from(path_str);
                        log::debug!("Extracted path from ambxst: {:?}", path);
                        if path.exists() {
                            return Some(path);
                        } else {
                            log::debug!("Path from ambxst does not exist: {:?}", path);
                        }
                    }
                }
            }
        }
        log::warn!("Could not extract currentWall from ambxst config");
        None
    }

    fn from_mpvpaper() -> Option<PathBuf> {
        let output = Command::new("pgrep").arg("-a").arg("mpvpaper").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in parts.iter().rev() {
                let path = PathBuf::from(part);
                if path.exists() && path.is_file() {
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
            if let Some(idx) = line.find("-i") {
                let rest = &line[idx + 2..].trim();
                if let Some(path_str) = rest.split_whitespace().next() {
                    let path = PathBuf::from(path_str);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    fn from_waypaper() -> Option<PathBuf> {
        let config_path = dirs::config_dir()?.join("waypaper").join("config.ini");
        if !config_path.exists() {
            return None;
        }
        let content = fs::read_to_string(config_path).ok()?;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("wallpaper") {
                if let Some((_, value)) = line.split_once('=') {
                    let path_str = value.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
                    let path = PathBuf::from(&path_str);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

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
                    let _ = fs::remove_file(temp_frame);
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

    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper() {
            Some(path) => path,
            None => {
                log::warn!("No wallpaper found, using default colors");
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

    pub fn start_wallpaper_monitor(tx: Sender<Vec<[f32; 4]>>, num_colors: usize) {
        thread::spawn(move || {
            let mut last_path: Option<PathBuf> = None;
            let mut last_modified: Option<std::time::SystemTime> = None;

            loop {
                if let Some(path) = Self::find_wallpaper() {
                    let modified = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let changed = match (&last_path, &last_modified) {
                        (Some(p), Some(t)) => p != &path || t != &modified,
                        _ => true,
                    };
                    if changed {
                        log::info!("Wallpaper changed to: {:?}", path);
                        match Self::generate_gradient_colors(num_colors) {
                            Ok(colors) => {
                                if let Err(e) = tx.send(colors) {
                                    log::error!("Failed to send new colors: {}", e);
                                    break;
                                }
                            }
                            Err(e) => log::error!("Failed to generate colors: {}", e),
                        }
                        last_path = Some(path);
                        last_modified = Some(modified);
                    }
                } else if last_path.is_some() {
                    log::warn!("Wallpaper disappeared, using default colors");
                    let default_colors = Self::default_colors(num_colors);
                    let _ = tx.send(default_colors);
                    last_path = None;
                    last_modified = None;
                }
                thread::sleep(Duration::from_secs(2));
            }
        });
    }
}