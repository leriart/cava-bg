#![allow(dead_code)]

use anyhow::{Context, Result};
use color_thief::{get_palette, ColorFormat};
use image::{self, GenericImageView};
use once_cell::sync::Lazy;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime};

use crate::app_config::ColorExtractionMode;
use crate::wallpaper_detector;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.7;

#[derive(Debug, Clone)]
pub struct WallpaperPlaybackInfo {
    pub media_path: PathBuf,
    pub daemon_name: String,
    pub process_pid: i32,
    pub elapsed_seconds: f64,
}

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    pub fn find_wallpaper() -> Option<PathBuf> {
        wallpaper_detector::get_current_wallpaper()
    }

    pub fn detect_playback_info() -> Option<WallpaperPlaybackInfo> {
        Self::detect_from_mpvpaper()
    }

    fn detect_from_mpvpaper() -> Option<WallpaperPlaybackInfo> {
        let output = Command::new("pgrep")
            .arg("-a")
            .arg("mpvpaper")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse::<i32>().ok()?;
            let args: Vec<&str> = parts.collect();
            let media = args
                .iter()
                .rev()
                .find(|arg| !arg.starts_with('-'))
                .map(PathBuf::from)?;
            if !media.exists() {
                continue;
            }
            let elapsed_seconds = Self::process_elapsed_seconds(pid).unwrap_or(0.0);
            return Some(WallpaperPlaybackInfo {
                media_path: media,
                daemon_name: "mpvpaper".to_string(),
                process_pid: pid,
                elapsed_seconds,
            });
        }

        None
    }

    fn process_elapsed_seconds(pid: i32) -> Option<f64> {
        let output = Command::new("ps")
            .args(["-o", "etimes=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        text.trim().parse::<f64>().ok()
    }

    fn load_image_from_path(path: &PathBuf) -> Result<image::DynamicImage> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "avi" | "mov") {
            let temp_frame = std::env::temp_dir().join("cava_bg_temp_frame.png");
            let status = Command::new("ffmpeg")
                .args([
                    "-i",
                    path.to_str().unwrap_or_default(),
                    "-vframes",
                    "1",
                    "-q:v",
                    "2",
                    "-update",
                    "1",
                    temp_frame.to_str().unwrap_or_default(),
                    "-y",
                ])
                .status();
            if let Ok(status) = status {
                if status.success() {
                    let img = image::open(&temp_frame).context("Failed to open video frame")?;
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

    pub fn extract_colors(
        wallpaper_path: &PathBuf,
        mode: ColorExtractionMode,
        num_colors: usize,
    ) -> Result<Vec<[f32; 4]>> {
        log::info!(
            "Analyzing wallpaper: {:?} with mode {:?}",
            wallpaper_path,
            mode
        );

        let img = Self::load_image_from_path(wallpaper_path)
            .with_context(|| format!("Could not load wallpaper image {:?}", wallpaper_path))?;

        let (width, height) = img.dimensions();
        log::debug!("Wallpaper dimensions: {}x{}", width, height);

        let rgb_img = img.to_rgb8();
        let pixels = rgb_img.as_raw();

        let mut new_colors: Vec<[f32; 4]> =
            get_palette(pixels, ColorFormat::Rgb, 10, num_colors as u8)
                .context("Failed to extract color palette")?
                .iter()
                .map(|c| {
                    [
                        c.r as f32 / 255.0,
                        c.g as f32 / 255.0,
                        c.b as f32 / 255.0,
                        1.0,
                    ]
                })
                .collect();

        match mode {
            ColorExtractionMode::Dominant | ColorExtractionMode::Palette => {
                new_colors.sort_by(|a, b| {
                    let lum_a = 0.299 * a[0] + 0.587 * a[1] + 0.114 * a[2];
                    let lum_b = 0.299 * b[0] + 0.587 * b[1] + 0.114 * b[2];
                    lum_a
                        .partial_cmp(&lum_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            ColorExtractionMode::Vibrant => {
                new_colors.sort_by(|a, b| {
                    let sat_a = (a[0].max(a[1]).max(a[2])) - (a[0].min(a[1]).min(a[2]));
                    let sat_b = (b[0].max(b[1]).max(b[2])) - (b[0].min(b[1]).min(b[2]));
                    sat_b
                        .partial_cmp(&sat_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }

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

        Ok(new_colors)
    }

    pub fn generate_gradient_colors(
        num_colors: usize,
        mode: Option<ColorExtractionMode>,
    ) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper() {
            Some(path) => path,
            None => {
                log::warn!("No wallpaper found, using default colors");
                return Ok(Self::default_colors(num_colors));
            }
        };

        let effective_mode = mode.unwrap_or(ColorExtractionMode::Dominant);
        match Self::extract_colors(&wallpaper_path, effective_mode, num_colors) {
            Ok(colors) => Ok(colors),
            Err(e) => {
                log::warn!(
                    "Could not extract colors from wallpaper: {}, using default colors",
                    e
                );
                Ok(Self::default_colors(num_colors))
            }
        }
    }

    pub fn start_wallpaper_monitor(
        tx: Sender<Vec<[f32; 4]>>,
        num_colors: usize,
        extraction_mode: ColorExtractionMode,
        extract_enabled: bool,
    ) {
        thread::spawn(move || {
            let mut last_path: Option<PathBuf> = None;
            let mut last_modified: Option<SystemTime> = None;
            let mut sent_initial = false;

            loop {
                // Only extract when extract_enabled is true OR if we haven't sent initial colors
                // (fallback for when parallax/xray have no wallpaper)
                let should_extract = extract_enabled || !sent_initial;

                if should_extract {
                    if let Some(path) = Self::find_wallpaper() {
                        let modified = fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        let changed = match (&last_path, &last_modified) {
                            (Some(p), Some(t)) => p != &path || t != &modified,
                            _ => true,
                        };
                        if changed {
                            log::info!("Wallpaper changed to: {:?}", path);
                            match Self::generate_gradient_colors(num_colors, Some(extraction_mode))
                            {
                                Ok(colors) => {
                                    if let Err(e) = tx.send(colors) {
                                        log::error!("Failed to send new colors: {}", e);
                                        break;
                                    }
                                    sent_initial = true;
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
                }
                // Check frequently for instant reaction
                thread::sleep(Duration::from_millis(100));
            }
        });
    }

    pub fn start_wallpaper_path_monitor(
        tx: Sender<Option<PathBuf>>,
        color_tx: Option<Sender<Vec<[f32; 4]>>>,
        num_colors: usize,
        extraction_mode: ColorExtractionMode,
    ) {
        thread::spawn(move || {
            let mut last_path: Option<PathBuf> = None;
            loop {
                let current_path = Self::find_wallpaper();
                if current_path != last_path {
                    log::info!("Wallpaper path changed: {:?}", current_path);
                    // Send path immediately so layers reload
                    if let Err(e) = tx.send(current_path.clone()) {
                        log::error!("Failed to send wallpaper path: {}", e);
                        break;
                    }
                    // Also trigger color extraction in the same pass
                    if let Some(ref color_tx) = color_tx {
                        if let Some(ref _path) = current_path {
                            log::info!("Extracting colors from new wallpaper");
                            match Self::generate_gradient_colors(num_colors, Some(extraction_mode))
                            {
                                Ok(colors) => {
                                    let _ = color_tx.send(colors);
                                }
                                Err(e) => log::error!("Failed to generate colors: {}", e),
                            }
                        }
                    }
                    last_path = current_path;
                }
                thread::sleep(Duration::from_millis(80));
            }
        });
    }
}
