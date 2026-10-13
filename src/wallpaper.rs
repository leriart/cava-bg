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
use std::time::{Duration, Instant, SystemTime};

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
        if num_colors == 0 {
            return catppuccin[0..1].to_vec();
        }
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

        // color_thief panics unless max_colors is in 2..=255, and requesting more
        // colors than distinct pixels can also fail — clamp defensively.
        let requested = num_colors.clamp(2, 255) as u8;
        let extracted: Vec<[f32; 4]> = get_palette(pixels, ColorFormat::Rgb, 10, requested)
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

        if extracted.is_empty() {
            anyhow::bail!("Palette extraction returned no colors");
        }

        // Match the requested count: truncate or repeat the last color to fill.
        let mut new_colors: Vec<[f32; 4]> = Vec::with_capacity(num_colors.max(1));
        for i in 0..num_colors.max(1) {
            new_colors.push(extracted[i.min(extracted.len() - 1)]);
        }

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
}

/// Cadence for the wallpaper watchers.
///
/// Wallpaper changes are not a sub-second event, but after one *is* observed we
/// poll quickly for a while so that a daemon which writes the image in pieces,
/// or a compositor that reports a half-written path, still converges quickly.
/// In steady state this drops the wakeups from ~12/s per thread to 1/s per
/// thread while keeping reaction time under ~100 ms right after a change.
fn poll_interval(stable_for: Duration) -> Duration {
    if stable_for < Duration::from_secs(5) {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(1000)
    }
}

/// Shared between the colour monitor and the path monitor.
///
/// Both threads watch the same wallpaper, so without this they each run their
/// own `find_wallpaper` loop *and* each perform palette extraction for the same
/// change. Two concurrent `extract_colors` calls also fight over the global
/// `PREVIOUS_COLORS` smoothing state.
#[derive(Default)]
struct WatchState {
    last_path: Option<PathBuf>,
    last_modified: Option<SystemTime>,
    last_seen_change: Option<Instant>,
    sent_initial: bool,
}

static WATCH_STATE: Lazy<Mutex<WatchState>> = Lazy::new(|| Mutex::new(WatchState::default()));

/// Result of one watcher tick.
#[derive(Debug, PartialEq)]
enum WatchTick {
    /// Nothing to do.
    Unchanged,
    /// The wallpaper changed (or appeared); `modified` may be `UNIX_EPOCH`.
    Changed { modified: SystemTime },
    /// The wallpaper that was being watched is gone.
    Disappeared,
}

/// Single detection point for both monitors.
///
/// Returns how long the state has been stable so the caller can pick its next
/// sleep interval.
fn watch_step() -> (WatchTick, Duration) {
    let now = Instant::now();
    let current = WallpaperAnalyzer::find_wallpaper();

    let mut state = match WATCH_STATE.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let stable_for = state
        .last_seen_change
        .map(|t| now.duration_since(t))
        .unwrap_or(Duration::ZERO);

    match current {
        Some(path) => {
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let changed = match (&state.last_path, &state.last_modified) {
                (Some(p), Some(t)) => p != &path || t != &modified,
                _ => true,
            };
            if changed {
                state.last_path = Some(path);
                state.last_modified = Some(modified);
                state.last_seen_change = Some(now);
                (WatchTick::Changed { modified }, Duration::ZERO)
            } else {
                (WatchTick::Unchanged, stable_for)
            }
        }
        None if state.last_path.is_some() => {
            state.last_path = None;
            state.last_modified = None;
            state.last_seen_change = Some(now);
            (WatchTick::Disappeared, Duration::ZERO)
        }
        None => (WatchTick::Unchanged, stable_for),
    }
}

impl WallpaperAnalyzer {
    pub fn start_wallpaper_monitor(
        tx: Sender<Vec<[f32; 4]>>,
        num_colors: usize,
        extraction_mode: ColorExtractionMode,
        extract_enabled: bool,
    ) {
        thread::spawn(move || loop {
            // Only extract when extract_enabled is true OR if we haven't sent initial colors
            // (fallback for when parallax/xray have no wallpaper)
            let wants_extraction = extract_enabled || !initial_colors_sent();

            match watch_step() {
                (WatchTick::Changed { .. }, _) if wants_extraction => {
                    match Self::generate_gradient_colors(num_colors, Some(extraction_mode)) {
                        Ok(colors) => {
                            if tx.send(colors).is_err() {
                                break;
                            }
                            mark_initial_colors_sent();
                        }
                        Err(e) => log::error!("Failed to generate colors: {}", e),
                    }
                }
                (WatchTick::Disappeared, _) => {
                    log::warn!("Wallpaper disappeared, using default colors");
                    let _ = tx.send(Self::default_colors(num_colors));
                }
                (_, stable_for) => thread::sleep(poll_interval(stable_for)),
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
            let mut reported: Option<Option<PathBuf>> = None;
            loop {
                let (tick, stable_for) = watch_step();

                match tick {
                    WatchTick::Changed { .. } => {
                        let path = current_wallpaper_path();
                        if reported.as_ref() != Some(&path) {
                            log::info!("Wallpaper path changed: {:?}", path);
                            if tx.send(path.clone()).is_err() {
                                break;
                            }
                            reported = Some(path.clone());
                        }

                        if let (Some(color_tx), Some(_)) = (color_tx.as_ref(), path.as_ref()) {
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
                    WatchTick::Disappeared => {
                        if reported.is_some() {
                            if tx.send(None).is_err() {
                                break;
                            }
                            reported = None;
                        }
                    }
                    WatchTick::Unchanged => {}
                }

                thread::sleep(poll_interval(stable_for));
            }
        });
    }
}

fn current_wallpaper_path() -> Option<PathBuf> {
    WATCH_STATE
        .lock()
        .map(|g| g.last_path.clone())
        .unwrap_or(None)
}

fn initial_colors_sent() -> bool {
    WATCH_STATE.lock().map(|g| g.sent_initial).unwrap_or(false)
}

fn mark_initial_colors_sent() {
    if let Ok(mut g) = WATCH_STATE.lock() {
        g.sent_initial = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_image(path: &PathBuf) {
        let mut img = image::RgbImage::new(8, 8);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = if (x + y) % 2 == 0 {
                image::Rgb([200, 30, 30])
            } else {
                image::Rgb([30, 30, 200])
            };
        }
        img.save(path).unwrap();
    }

    #[test]
    fn extract_colors_does_not_panic_for_edge_counts() {
        let dir = std::env::temp_dir();
        let path = dir.join("cava_bg_test_wallpaper.png");
        write_test_image(&path);

        for n in [0usize, 1, 2, 8, 300] {
            let colors = WallpaperAnalyzer::extract_colors(&path, ColorExtractionMode::Dominant, n)
                .unwrap_or_else(|e| panic!("extract_colors failed for n={}: {}", n, e));
            assert_eq!(colors.len(), n.max(1), "wrong color count for n={}", n);
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn poll_interval_backs_off_only_after_a_stable_window() {
        // Right after a change we want a fast tick so a partially written
        // wallpaper converges quickly.
        assert_eq!(poll_interval(Duration::ZERO), Duration::from_millis(100));
        assert_eq!(
            poll_interval(Duration::from_secs(4)),
            Duration::from_millis(100)
        );
        // Steady state: a wallpaper does not change 12 times a second.
        assert_eq!(
            poll_interval(Duration::from_secs(5)),
            Duration::from_millis(1000)
        );
        assert_eq!(
            poll_interval(Duration::from_secs(3600)),
            Duration::from_millis(1000)
        );
    }

    #[test]
    fn watch_step_settles_instead_of_repeating_a_change() {
        // This test deliberately does not assume whether a wallpaper is
        // discoverable on the machine running it. It asserts the invariant that
        // matters: whatever the first tick reports, the state converges and
        // subsequent ticks stop reporting changes.
        let reset = WATCH_STATE.lock().unwrap();
        let mut guard = reset;
        *guard = WatchState::default();
        drop(guard);

        let first = watch_step().0;
        for _ in 0..3 {
            assert_eq!(
                watch_step().0,
                WatchTick::Unchanged,
                "watcher kept emitting change events after {first:?}"
            );
        }
    }

    #[test]
    fn default_colors_handles_zero() {
        assert_eq!(WallpaperAnalyzer::default_colors(0).len(), 1);
        assert_eq!(WallpaperAnalyzer::default_colors(8).len(), 8);
        assert_eq!(WallpaperAnalyzer::default_colors(16).len(), 16);
    }
}
