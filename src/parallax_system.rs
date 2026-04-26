#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use log::{info, warn};
use crate::app_config::{
    AnimationType, AudioConfig, AudioResponseCurve, BlendMode, FrequencyZone, LayerSourceType,
    ParallaxConfig, ParallaxEffectType, ParallaxLayerConfig, ParallaxMouseConfig, ParallaxProfile,
    ProfileSource,
};
use crate::video_decoder::{VideoDecoder, VideoDecoderConfig, VideoFrame};

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioBands {
    pub amplitude: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
}

impl AudioBands {
    pub fn from_bars(bars: &[f32], peak: f32) -> Self {
        if bars.is_empty() {
            return Self {
                amplitude: peak.clamp(0.0, 1.0),
                ..Self::default()
            };
        }

        let len = bars.len();
        let third = (len / 3).max(1);

        let low = bars[..third].iter().copied().sum::<f32>() / third as f32;
        let mid_slice = &bars[third..(third * 2).min(len)];
        let mid = if mid_slice.is_empty() {
            0.0
        } else {
            mid_slice.iter().copied().sum::<f32>() / mid_slice.len() as f32
        };
        let high_slice = &bars[(third * 2).min(len)..];
        let high = if high_slice.is_empty() {
            0.0
        } else {
            high_slice.iter().copied().sum::<f32>() / high_slice.len() as f32
        };

        Self {
            amplitude: peak.clamp(0.0, 1.0),
            low: low.clamp(0.0, 1.0),
            mid: mid.clamp(0.0, 1.0),
            high: high.clamp(0.0, 1.0),
        }
    }

    fn zone_value(&self, zone: FrequencyZone) -> f32 {
        match zone {
            FrequencyZone::FullSpectrum => self.amplitude,
            FrequencyZone::Low => self.low,
            FrequencyZone::Mid => self.mid,
            FrequencyZone::High => self.high,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LayerRenderTransform {
    pub translation_px: [f32; 2],
    pub scale: f32,
    pub rotation_rad: f32,
    pub opacity: f32,
}

impl Default for LayerRenderTransform {
    fn default() -> Self {
        Self {
            translation_px: [0.0, 0.0],
            scale: 1.0,
            rotation_rad: 0.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComputedParallaxLayer {
    pub id: usize,
    pub name: String,
    pub blend_mode: BlendMode,
    pub depth: f32,
    pub z_index: i32,
    pub transform: LayerRenderTransform,
    pub frame: Option<VideoFrame>,
}

#[derive(Debug)]
enum LayerAsset {
    Static(VideoFrame),
    Dynamic(VideoDecoder),
    Effect,
}

#[derive(Debug)]
struct RuntimeLayer {
    id: usize,
    cfg: ParallaxLayerConfig,
    source: PathBuf,
    inferred_type: LayerSourceType,
    asset: Option<LayerAsset>,
    last_frame: Option<VideoFrame>,
}

impl RuntimeLayer {
    fn frame_for_time(&mut self, audio_cfg: Option<&AudioConfig>, playback_time: f64, audio: AudioBands) -> Option<VideoFrame> {
        match self.asset.as_mut() {
            Some(LayerAsset::Static(frame)) => {
                self.last_frame = Some(frame.clone());
                self.last_frame.clone()
            }
            Some(LayerAsset::Dynamic(decoder)) => {
                let maybe_frame = if let Some(duration) = decoder.stream_duration_seconds() {
                    if duration > 0.0 {
                        decoder.poll_latest_for_time(playback_time.rem_euclid(duration))
                    } else {
                        decoder.poll_latest_for_time(playback_time)
                    }
                } else {
                    decoder.poll_latest_for_time(playback_time)
                };

                if let Some(frame) = maybe_frame {
                    self.last_frame = Some(frame.clone());
                    Some(frame)
                } else {
                    self.last_frame.clone()
                }
            }
            Some(LayerAsset::Effect) => {
                let frame = self.generate_effect_frame(audio_cfg, playback_time, audio);
                self.last_frame = Some(frame.clone());
                Some(frame)
            }
            None => None,
        }
    }

    fn generate_effect_frame(&self, audio_cfg: Option<&AudioConfig>, playback_time: f64, audio: AudioBands) -> VideoFrame {
        let width = 640u32;
        let height = 360u32;
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let effect = self.cfg.effect.clone().unwrap_or_default();
        let bars = if effect.bars > 0 {
            effect.bars as usize
        } else if let Some(cfg) = audio_cfg {
            cfg.bar_count.max(8) as usize
        } else {
            32usize
        };
        let tint = effect.tint;
        let gap = effect.gap.max(0.0);
        let height_scale = effect.height_scale.max(0.1);
        let amp = audio.amplitude.clamp(0.0, 1.0);
        let t = playback_time as f32;
        let bar_width = width as f32 / (bars as f32 * (1.0 + gap));

        match effect.effect_type {
            ParallaxEffectType::CavaBars => {
                for i in 0..bars {
                    let x0 = (i as f32 * (bar_width + bar_width * gap)) as usize;
                    let x1 = ((x0 as f32 + bar_width) as usize).min(width as usize);
                    let zone = if i < bars / 3 {
                        audio.low
                    } else if i < bars * 2 / 3 {
                        audio.mid
                    } else {
                        audio.high
                    };
                    let wobble = ((t * 2.2) + i as f32 * 0.35).sin() * 0.15;
                    let h_norm = (zone * 0.8 + amp * 0.2 + wobble.abs() * 0.3).clamp(0.02, 1.0);
                    let h_px = ((h_norm * height_scale * height as f32) as usize).min(height as usize);
                    for y in 0..h_px {
                        let yy = (height as usize - 1).saturating_sub(y);
                        for x in x0..x1 {
                            let idx = (yy * width as usize + x) * 4;
                            rgba[idx] = (tint[0] * 255.0) as u8;
                            rgba[idx + 1] = (tint[1] * 255.0) as u8;
                            rgba[idx + 2] = (tint[2] * 255.0) as u8;
                            rgba[idx + 3] = ((tint[3] * 255.0) * (0.5 + 0.5 * h_norm)) as u8;
                        }
                    }
                }
            }
            ParallaxEffectType::CavaWave => {
                let center = height as f32 * 0.5;
                for x in 0..width as usize {
                    let phase = x as f32 / width as f32 * std::f32::consts::TAU;
                    let y = center + (phase * 4.0 + t * 5.0).sin() * (20.0 + 120.0 * amp);
                    for dy in -2..=2 {
                        let yy = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
                        let idx = (yy * width as usize + x) * 4;
                        rgba[idx] = (tint[0] * 255.0) as u8;
                        rgba[idx + 1] = (tint[1] * 255.0) as u8;
                        rgba[idx + 2] = (tint[2] * 255.0) as u8;
                        rgba[idx + 3] = (tint[3] * 255.0) as u8;
                    }
                }
            }
            ParallaxEffectType::CavaRadial => {
                let cx = width as f32 * 0.5;
                let cy = height as f32 * 0.5;
                let base = 40.0;
                for i in 0..bars {
                    let angle = i as f32 / bars as f32 * std::f32::consts::TAU + t * 0.3;
                    let zone = if i < bars / 3 {
                        audio.low
                    } else if i < bars * 2 / 3 {
                        audio.mid
                    } else {
                        audio.high
                    };
                    let len = base + zone * 140.0 + amp * 40.0;
                    let x1 = cx + angle.cos() * len;
                    let y1 = cy + angle.sin() * len;
                    draw_line_rgba(
                        &mut rgba,
                        width as usize,
                        height as usize,
                        cx,
                        cy,
                        x1,
                        y1,
                        tint,
                    );
                }
            }
        }

        VideoFrame {
            rgba,
            width,
            height,
            pts_seconds: playback_time,
            duration_seconds: 1.0 / 60.0,
            sequence: (playback_time * 1000.0) as u64,
        }
    }
}

#[derive(Debug)]
pub struct ParallaxSystem {
    cfg: ParallaxConfig,
    target_size: (u32, u32),
    created_at: Instant,
    layers: Vec<RuntimeLayer>,
    global_mouse: (f32, f32),
    per_output_mouse: HashMap<String, (f32, f32)>,
    wallpaper_name: Option<String>,
}

impl ParallaxSystem {
    pub fn new(cfg: ParallaxConfig, target_width: u32, target_height: u32, wallpaper_name: Option<String>) -> Result<Self> {
        let mut instance = Self {
            cfg,
            target_size: (target_width.max(1), target_height.max(1)),
            created_at: Instant::now(),
            layers: Vec::new(),
            global_mouse: (0.5, 0.5),
            per_output_mouse: HashMap::new(),
            wallpaper_name,
        };
        instance.rebuild_layers()?;
        Ok(instance)
    }

    pub fn set_config(&mut self, cfg: ParallaxConfig) -> Result<()> {
        self.cfg = cfg;
        self.rebuild_layers()
    }

    pub fn set_target_size(&mut self, width: u32, height: u32) {
        self.target_size = (width.max(1), height.max(1));
    }

    pub fn set_wallpaper_name(&mut self, name: Option<String>) {
        self.wallpaper_name = name;
    }

    pub fn wallpaper_name(&self) -> Option<&str> {
        self.wallpaper_name.as_deref()
    }

    pub fn on_wallpaper_change(&mut self, name: Option<String>) -> Result<()> {
        info!("[PARALLAX] on_wallpaper_change called, name={:?}, current layers={}", name, self.layers.len());
        self.wallpaper_name = name;
        self.rebuild_layers()?;
        info!("[PARALLAX] after rebuild_layers, layers={}", self.layers.len());
        Ok(())
    }

    pub fn set_mouse_global(&mut self, x: f32, y: f32) {
        self.global_mouse = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    }

    pub fn set_mouse_for_output(&mut self, output: &str, x: f32, y: f32) {
        self.per_output_mouse
            .insert(output.to_string(), (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
    }

    pub fn remove_output_mouse(&mut self, output: &str) {
        self.per_output_mouse.remove(output);
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled && !self.layers.is_empty()
    }

    pub fn compute_layers(
        &mut self,
        output_name: &str,
        audio: AudioBands,
        audio_cfg: Option<&AudioConfig>,
        is_idle: bool,
        avg_frame_time_ms: Option<f32>,
    ) -> Vec<ComputedParallaxLayer> {
        if !self.cfg.enabled {
            return Vec::new();
        }

        if is_idle && self.cfg.performance.pause_on_idle {
            return Vec::new();
        }

        if self.cfg.performance.disable_under_load {
            if let Some(avg_ms) = avg_frame_time_ms {
                if avg_ms > self.cfg.performance.frame_time_budget_ms.max(1.0) {
                    return Vec::new();
                }
            }
        }

        let playback_time = self.created_at.elapsed().as_secs_f64();
        let mouse = self.resolve_mouse_for_output(output_name);
        let mouse_cfg = self.cfg.mouse.clone();
        let enable_3d_depth = self.cfg.enable_3d_depth;

        let mut out = Vec::with_capacity(self.layers.len());
        for layer in &mut self.layers {
            let is_effect = layer
                .cfg
                .effect
                .as_ref()
                .map(|e| e.enabled)
                .unwrap_or(false);
            if !layer.cfg.enabled || (!is_effect && layer.source.as_os_str().is_empty()) {
                continue;
            }

            if layer.asset.is_none() {
                if self.cfg.performance.lazy_load_assets {
                    if Self::try_load_layer_asset(layer, self.target_size).is_err() {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            let frame = layer.frame_for_time(audio_cfg, playback_time, audio);
            if frame.is_none() {
                continue;
            }

            let transform = Self::compute_transform(
                &layer.cfg,
                &mouse_cfg,
                enable_3d_depth,
                mouse,
                audio,
                playback_time as f32,
            );
            out.push(ComputedParallaxLayer {
                id: layer.id,
                name: if layer.cfg.name.trim().is_empty() {
                    format!("layer-{}", layer.id)
                } else {
                    layer.cfg.name.clone()
                },
                blend_mode: layer.cfg.blend_mode,
                depth: layer.cfg.depth.clamp(0.0, 1.0),
                z_index: layer.cfg.z_index,
                transform,
                frame,
            });
        }

        out.sort_by(|a, b| {
            a.depth
                .partial_cmp(&b.depth)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.z_index.cmp(&b.z_index))
        });

        out
    }

    fn resolve_mouse_for_output(&self, output_name: &str) -> (f32, f32) {
        let use_per_output = self.cfg.mouse.per_output_tracking;
        let use_global = self.cfg.mouse.global_tracking;

        if !self.cfg.mouse.enabled {
            return (0.5, 0.5);
        }

        match (use_per_output, use_global) {
            (true, true) => {
                if let Some(per_output) = self.per_output_mouse.get(output_name) {
                    (
                        (per_output.0 + self.global_mouse.0) * 0.5,
                        (per_output.1 + self.global_mouse.1) * 0.5,
                    )
                } else {
                    self.global_mouse
                }
            }
            (true, false) => self
                .per_output_mouse
                .get(output_name)
                .copied()
                .unwrap_or((0.5, 0.5)),
            (false, true) => self.global_mouse,
            (false, false) => (0.5, 0.5),
        }
    }

    fn compute_transform(
        layer: &ParallaxLayerConfig,
        mouse_cfg: &ParallaxMouseConfig,
        enable_3d_depth: bool,
        mouse: (f32, f32),
        audio: AudioBands,
        elapsed: f32,
    ) -> LayerRenderTransform {
        let mut transform = LayerRenderTransform {
            translation_px: layer.offset,
            opacity: layer.opacity.clamp(0.0, 1.0),
            ..LayerRenderTransform::default()
        };

        if mouse_cfg.enabled && layer.mouse.enabled {
            // Mouse in [-1, 1] range
            let cx = (mouse.0 - 0.5) * 2.0;
            let cy = (mouse.1 - 0.5) * 2.0;

            // NDC displacement: layer Z INDEX controls how much of the screen it shifts.
            // Each increment of z doubles the displacement (exponential parallax).
            let z_power = (2.0_f32).powi(layer.z_index.max(0));

            // User multipliers for overall intensity
            let depth_scale = if enable_3d_depth {
                layer.depth.clamp(0.0, 1.0)
            } else {
                1.0
            };
            let gain = mouse_cfg.sensitivity * layer.mouse.sensitivity * layer.parallax_speed;
            let offset_scale = (layer.mouse.max_offset[0] / 32.0).max(0.25);

            // translation_px stores NDC offset (fraction of screen, -1 to 1 range).
            // We want strong horizontal parallax and weak vertical parallax.
            // z=0: displacement_x = 0.001, barely visible
            // z=1: displacement_x = 0.002
            // z=2: displacement_x = 0.004
            // z=3: displacement_x = 0.008
            // z=4: displacement_x = 0.016
            // Vertical is 1/4 of horizontal for a more natural depth feel.
            let ndc_x = cx * 0.001 * z_power * depth_scale * gain * offset_scale;
            let ndc_y = cy * 0.00025 * z_power.min(4.0) * depth_scale * gain * offset_scale;

            // Apply the NDC offset directly — from_layer() will pass this through as-is
            transform.translation_px[0] += ndc_x;
            transform.translation_px[1] += ndc_y;
        }

        if layer.audio.enabled {
            let zone = audio.zone_value(layer.audio.frequency_zone);
            let mut intensity = (audio.amplitude * layer.audio.amplitude_sensitivity)
                + (zone * layer.audio.frequency_sensitivity);
            intensity *= 0.5;
            let mapped = map_audio_curve(intensity, layer.audio.response_curve);

            if layer.audio.transform.shift {
                transform.translation_px[1] -= mapped * layer.audio.transform.shift_amount;
            }
            if layer.audio.transform.scale {
                transform.scale += mapped * layer.audio.transform.scale_amount;
            }
            if layer.audio.transform.rotate {
                transform.rotation_rad +=
                    (mapped * layer.audio.transform.rotation_amount).to_radians();
            }
            transform.opacity = (transform.opacity + mapped * 0.2).clamp(0.0, 1.0);
        }

        if let Some(animation) = &layer.animation {
            if animation.enabled {
                let phase = elapsed * animation.speed.max(0.01);
                match animation.animation_type {
                    AnimationType::Float => {
                        transform.translation_px[1] += phase.sin() * animation.amplitude;
                    }
                    AnimationType::Rotate => {
                        transform.rotation_rad += (phase.sin() * animation.amplitude).to_radians();
                    }
                    AnimationType::Scale => {
                        transform.scale += (phase.sin().abs()) * (animation.amplitude * 0.02);
                    }
                    AnimationType::Pulse => {
                        transform.opacity *= (0.75 + 0.25 * phase.sin().abs()).clamp(0.0, 1.0);
                    }
                    AnimationType::Wiggle => {
                        transform.translation_px[0] += phase.sin() * animation.amplitude;
                        transform.translation_px[1] += (phase * 1.3).cos() * animation.amplitude;
                    }
                }
            }
        }

        transform
    }

    fn rebuild_layers(&mut self) -> Result<()> {
        self.layers.clear();

        // Determine profiles directory: from config, or default to ~/.config/cava-bg/parallax
        let profiles_dir = self.cfg.profiles_dir.clone()
            .or_else(|| {
                dirs::config_dir().map(|d| d.join("cava-bg").join("parallax"))
            })
            .filter(|d| d.exists());

        if let Some(ref pd) = profiles_dir {
            // Determine which profile to load
            let active: Option<String> = match self.cfg.profile_source {
                ProfileSource::FromWallpaper => self.wallpaper_name.clone(),
                ProfileSource::Normal => self.cfg.active_profile.clone(),
            };

            if let Some(ref profile_name) = active {
                if let Ok(profile) = ParallaxProfile::load(pd, profile_name) {
                    for layer_file in &profile.layers {
                        let source = profile.resolve_layer(pd, layer_file);
                        let mut layer_cfg = profile.layer_config(layer_file);
                        if layer_cfg.source.trim().is_empty() {
                            layer_cfg.source = source.to_string_lossy().to_string();
                        }
                        self.add_layer_from_cfg(layer_cfg)?;
                    }
                    return Ok(());
                }
                // Profile not found — if FromWallpaper mode, show nothing (no layers)
                if self.cfg.profile_source == ProfileSource::FromWallpaper {
                    return Ok(());
                }
            }

            // Only auto-select first profile if no wallpaper name is set
            if active.is_none() && self.wallpaper_name.is_none() && self.cfg.layers.is_empty() {
                // No active profile and no explicit layers — scan for available profiles
                // and pick first one
                let profiles = ParallaxProfile::discover_profiles(pd);
                if let Some(first) = profiles.first() {
                    if let Ok(profile) = ParallaxProfile::load(pd, first) {
                        for layer_file in &profile.layers {
                            let source = profile.resolve_layer(pd, layer_file);
                            let mut layer_cfg = profile.layer_config(layer_file);
                            if layer_cfg.source.trim().is_empty() {
                                layer_cfg.source = source.to_string_lossy().to_string();
                            }
                            self.add_layer_from_cfg(layer_cfg)?;
                        }
                        warn!("No active parallax profile set, auto-selected '{}'", first);
                        return Ok(());
                    }
                }
            }
        }

        // Fallback: load explicit layers from config
        for (idx, layer_cfg) in self.cfg.layers.iter().enumerate() {
            let is_effect = layer_cfg
                .effect
                .as_ref()
                .map(|e| e.enabled)
                .unwrap_or(false);
            if layer_cfg.source.trim().is_empty() && !is_effect {
                continue;
            }

            let source = PathBuf::from(layer_cfg.source.trim());
            let inferred_type = layer_cfg
                .layer_type
                .unwrap_or_else(|| infer_layer_type(&source));

            let mut runtime = RuntimeLayer {
                id: idx,
                cfg: layer_cfg.clone(),
                source,
                inferred_type,
                asset: None,
                last_frame: None,
            };

            if !self.cfg.performance.lazy_load_assets {
                Self::try_load_layer_asset(&mut runtime, self.target_size)?;
            }

            self.layers.push(runtime);
        }

        Ok(())
    }

    fn add_layer_from_cfg(&mut self, layer_cfg: ParallaxLayerConfig) -> Result<()> {
        let idx = self.layers.len();
        let is_effect = layer_cfg
            .effect
            .as_ref()
            .map(|e| e.enabled)
            .unwrap_or(false);
        if layer_cfg.source.trim().is_empty() && !is_effect {
            return Ok(());
        }

        let source = PathBuf::from(layer_cfg.source.trim());
        let inferred_type = layer_cfg
            .layer_type
            .unwrap_or_else(|| infer_layer_type(&source));

        let mut runtime = RuntimeLayer {
            id: idx,
            cfg: layer_cfg,
            source,
            inferred_type,
            asset: None,
            last_frame: None,
        };

        if !self.cfg.performance.lazy_load_assets {
            Self::try_load_layer_asset(&mut runtime, self.target_size)?;
        }

        self.layers.push(runtime);
        Ok(())
    }

    fn try_load_layer_asset(layer: &mut RuntimeLayer, target_size: (u32, u32)) -> Result<()> {
        if layer.asset.is_some() {
            return Ok(());
        }

        if layer
            .cfg
            .effect
            .as_ref()
            .map(|e| e.enabled)
            .unwrap_or(false)
        {
            layer.asset = Some(LayerAsset::Effect);
            return Ok(());
        }

        match layer.inferred_type {
            LayerSourceType::StaticImage => {
                let frame = decode_static_image(&layer.source)
                    .with_context(|| {
                        format!(
                            "Failed to decode static parallax layer {}",
                            layer.source.display()
                        )
                    })?;
                layer.asset = Some(LayerAsset::Static(frame.clone()));
                layer.last_frame = Some(frame);
            }
            LayerSourceType::Video | LayerSourceType::Gif => {
                let decoder = VideoDecoder::new(
                    &layer.source,
                    VideoDecoderConfig {
                        target_width: target_size.0,
                        target_height: target_size.1,
                        looping: true,
                        max_buffered_frames: 6,
                        frame_cache_size: 120,
                    },
                )
                .with_context(|| {
                    format!(
                        "Failed to create decoder for parallax layer {}",
                        layer.source.display()
                    )
                })?;
                layer.asset = Some(LayerAsset::Dynamic(decoder));
            }
        }

        Ok(())
    }
}

fn infer_layer_type(path: &Path) -> LayerSourceType {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "gif" => LayerSourceType::Gif,
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" | "flv" | "wmv" => LayerSourceType::Video,
        _ => LayerSourceType::StaticImage,
    }
}

fn decode_static_image(path: &Path) -> Result<VideoFrame> {
    let img = image::open(path).with_context(|| format!("Cannot open image {}", path.display()))?;
    let rgba = img.to_rgba8();
    let ow = rgba.width().max(1);
    let oh = rgba.height().max(1);

    // Load at original size — no resize, no crop, no padding.
    // The parallax shader handles aspect ratio preservation via
    // crop_scale/crop_offset, exactly like X-Ray does.
    Ok(VideoFrame {
        rgba: rgba.into_raw(),
        width: ow,
        height: oh,
        pts_seconds: 0.0,
        duration_seconds: 1.0 / 60.0,
        sequence: 0,
    })
}

fn map_audio_curve(value: f32, curve: AudioResponseCurve) -> f32 {
    let v = value.clamp(0.0, 1.0);
    match curve {
        AudioResponseCurve::Linear => v,
        AudioResponseCurve::Smooth => v * v * (3.0 - 2.0 * v),
        AudioResponseCurve::Exponential => v.powf(2.2),
        AudioResponseCurve::Punchy => v.powf(0.6),
    }
}

fn draw_line_rgba(
    rgba: &mut [u8],
    width: usize,
    height: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    tint: [f32; 4],
) {
    let mut x0 = x0 as i32;
    let mut y0 = y0 as i32;
    let x1 = x1 as i32;
    let y1 = y1 as i32;

    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && x0 < width as i32 && y0 >= 0 && y0 < height as i32 {
            let idx = (y0 as usize * width + x0 as usize) * 4;
            rgba[idx] = (tint[0] * 255.0) as u8;
            rgba[idx + 1] = (tint[1] * 255.0) as u8;
            rgba[idx + 2] = (tint[2] * 255.0) as u8;
            rgba[idx + 3] = (tint[3] * 255.0) as u8;
        }

        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
