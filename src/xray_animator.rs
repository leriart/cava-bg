// Lightweight X-Ray animator.
//
// It does NOT decode videos itself: the X-Ray image is drawn by the existing
// WGPU pipeline. This module only computes a single `intensity_modulation`
// [0..1] value per frame, driven by the selected animation type.
//
// When animation_type = WallpaperSync and the current wallpaper is an
// animated file (mp4/gif/webm/mkv/mov/avi), we track the elapsed time
// versus an estimated loop duration to derive a synced phase. If duration
// cannot be detected (no ffprobe), we fall back to a 5-second loop.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app_config::{XRayAnimationType, XRayConfig};

pub struct WallpaperFrameTracker {
    pub loop_duration_secs: f32,
    pub start_time: Instant,
}

impl WallpaperFrameTracker {
    pub fn new(duration_secs: f32) -> Self {
        Self {
            loop_duration_secs: duration_secs.max(0.1),
            start_time: Instant::now(),
        }
    }

    /// Returns phase in [0, 1) representing the current position within the loop.
    #[inline]
    pub fn phase(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        (elapsed / self.loop_duration_secs).fract()
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.start_time = Instant::now();
    }
}

pub fn is_animated_wallpaper(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "mp4" | "webm" | "mkv" | "gif" | "mov" | "avi" | "m4v"
        ),
        None => false,
    }
}

/// Best-effort probe of an animated wallpaper's loop duration (seconds).
/// Uses `ffprobe` if available. Falls back to 5.0 on failure.
pub fn probe_wallpaper_duration(path: &Path) -> f32 {
    // Try ffprobe: `ffprobe -v error -show_entries format=duration -of csv=p=0 <file>`
    if let Ok(output) = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
    {
        if output.status.success() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Ok(d) = s.trim().parse::<f32>() {
                    if d.is_finite() && d > 0.0 {
                        return d;
                    }
                }
            }
        }
    }
    5.0
}

pub struct XRayAnimator {
    pub tracker: Option<WallpaperFrameTracker>,
    last_wallpaper: Option<PathBuf>,
    clock_start: Instant,
    audio_env: f32, // smoothed envelope
}

impl Default for XRayAnimator {
    fn default() -> Self {
        Self {
            tracker: None,
            last_wallpaper: None,
            clock_start: Instant::now(),
            audio_env: 0.0,
        }
    }
}

impl XRayAnimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh the wallpaper tracker if the wallpaper changed.
    pub fn on_wallpaper_change(&mut self, path: Option<&PathBuf>, cfg: &XRayConfig) {
        self.last_wallpaper = path.cloned();
        if !cfg.animation_enabled
            || cfg.animation_type != XRayAnimationType::WallpaperSync
            || !cfg.auto_detect_wallpaper_fps
        {
            self.tracker = None;
            return;
        }
        self.tracker = match path {
            Some(p) if is_animated_wallpaper(p) => {
                let dur = probe_wallpaper_duration(p);
                Some(WallpaperFrameTracker::new(dur))
            }
            _ => None,
        };
    }

    /// Compute the intensity multiplier in [0,1] for this frame.
    /// `audio_peak` should be in [0,1].
    pub fn update(&mut self, cfg: &XRayConfig, audio_peak: f32) -> f32 {
        if !cfg.enabled || !cfg.animation_enabled {
            return 1.0;
        }

        let t = self.clock_start.elapsed().as_secs_f32() * cfg.animation_speed.max(0.001);

        // Simple leaky integrator for audio envelope, avoids flicker.
        self.audio_env = self.audio_env * 0.82 + audio_peak.clamp(0.0, 1.0) * 0.18;

        let m = match cfg.animation_type {
            XRayAnimationType::None => 1.0,
            XRayAnimationType::Fade => {
                // Cosine fade in/out in 0.5..1.0
                0.5 + 0.5 * (t.sin() * 0.5 + 0.5)
            }
            XRayAnimationType::Pulse => {
                // Sharper pulse around beat
                let s = (t * std::f32::consts::TAU).sin();
                0.5 + 0.5 * (s.max(0.0)).powf(2.0)
            }
            XRayAnimationType::WaveReveal => {
                // Sweeping wave: constant 1 but modulated with triangle wave
                let phase = (t * 0.5).fract();
                if phase < 0.5 {
                    phase * 2.0
                } else {
                    2.0 - phase * 2.0
                }
            }
            XRayAnimationType::AudioSync => {
                let k = (cfg.audio_sensitivity.max(0.0)) * self.audio_env;
                k.clamp(0.0, 1.0)
            }
            XRayAnimationType::WallpaperSync => {
                if let Some(tracker) = &self.tracker {
                    // Sine of wallpaper loop phase for smooth, synced modulation.
                    let phase = tracker.phase();
                    0.5 + 0.5 * (phase * std::f32::consts::TAU).sin()
                } else {
                    // No animated wallpaper detected, fall back to fade.
                    0.5 + 0.5 * t.sin()
                }
            }
        };

        (m * cfg.intensity.max(0.0)).clamp(0.0, 1.5)
    }
}
