#![allow(dead_code)]

use crate::app_config::{BlendMode, LayerConfig, LayerSourceType, XrayMaskConfig};
use crate::video_decoder::{VideoDecoder, VideoDecoderConfig, VideoFrame};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerType {
    StaticImage,
    Video,
    Gif,
}

#[derive(Debug)]
pub struct Layer {
    pub name: String,
    pub layer_type: LayerType,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub texture_size: (u32, u32),
    pub decoder: Option<VideoDecoder>,
    pub current_frame: Option<VideoFrame>,
}

#[derive(Debug)]
pub struct LayerManager {
    pub base: Layer,
    pub reveal: Layer,
    playback_start: Instant,
    playback_offset_seconds: f64,
    pub mask_engine: FingerprintMaskEngine,
    pub xray_background_color: Option<[f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct LayerUpdate {
    pub base_frame: Option<VideoFrame>,
    pub reveal_frame: Option<VideoFrame>,
    pub mask_rgba: Option<Vec<u8>>,
    pub mask_size: (u32, u32),
    pub xray_background_color: Option<[f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct FingerprintMaskEngine {
    prev_base_luma: Option<Vec<f32>>,
    prev_reveal_luma: Option<Vec<f32>>,
    width: u32,
    height: u32,
    cfg: XrayMaskConfig,
}

impl FingerprintMaskEngine {
    pub fn new(width: u32, height: u32, cfg: XrayMaskConfig) -> Self {
        Self {
            prev_base_luma: None,
            prev_reveal_luma: None,
            width,
            height,
            cfg,
        }
    }

    pub fn generate_mask(&mut self, base: &VideoFrame, reveal: &VideoFrame) -> Vec<u8> {
        let total = (self.width * self.height) as usize;
        let mut curr_base = vec![0.0f32; total];
        let mut curr_reveal = vec![0.0f32; total];
        let mut mask = vec![0u8; total * 4];

        for i in 0..total {
            let bi = i * 4;
            let base_luma = luma(base.rgba[bi], base.rgba[bi + 1], base.rgba[bi + 2]);
            let reveal_luma = luma(reveal.rgba[bi], reveal.rgba[bi + 1], reveal.rgba[bi + 2]);
            curr_base[i] = base_luma;
            curr_reveal[i] = reveal_luma;

            let spatial_similarity = 1.0 - (base_luma - reveal_luma).abs();

            // Fingerprint from stable objects: where pixels change slowly over time
            // and are also structurally similar between base/reveal layers.
            let temporal_stability_base = self
                .prev_base_luma
                .as_ref()
                .map(|prev| 1.0 - (base_luma - prev[i]).abs())
                .unwrap_or(1.0);
            let temporal_stability_reveal = self
                .prev_reveal_luma
                .as_ref()
                .map(|prev| 1.0 - (reveal_luma - prev[i]).abs())
                .unwrap_or(1.0);

            let mut fingerprint = 0.55 * temporal_stability_base
                + 0.25 * temporal_stability_reveal
                + 0.20 * spatial_similarity;
            fingerprint = fingerprint.clamp(0.0, 1.0);

            let thresholded = smoothstep(self.cfg.intensity, 1.0, fingerprint);
            let alpha = thresholded.powf(self.cfg.gamma.max(0.01));
            let alpha_u8 = (alpha.clamp(0.0, 1.0) * 255.0) as u8;

            mask[bi] = alpha_u8;
            mask[bi + 1] = alpha_u8;
            mask[bi + 2] = alpha_u8;
            mask[bi + 3] = 255;
        }

        self.prev_base_luma = Some(curr_base);
        self.prev_reveal_luma = Some(curr_reveal);
        mask
    }
}

impl LayerManager {
    pub fn new(
        base_cfg: &LayerConfig,
        reveal_cfg: &LayerConfig,
        width: u32,
        height: u32,
        mask_cfg: XrayMaskConfig,
        playback_offset_seconds: f64,
    ) -> Result<Self> {
        let xray_background_color = if mask_cfg.use_background {
            mask_cfg.xray_background_color
        } else {
            None
        };

        let base = Self::build_layer("base", base_cfg, width, height)?;
        let reveal = Self::build_layer("reveal", reveal_cfg, width, height)?;
        Ok(Self {
            base,
            reveal,
            playback_start: Instant::now(),
            playback_offset_seconds,
            mask_engine: FingerprintMaskEngine::new(width, height, mask_cfg),
            xray_background_color,
        })
    }

    pub fn playback_time_seconds(&self) -> f64 {
        self.playback_start.elapsed().as_secs_f64() + self.playback_offset_seconds
    }

    pub fn update(&mut self) -> LayerUpdate {
        let playback_time = self.playback_time_seconds();

        let base_frame = pull_layer_frame(&mut self.base, playback_time);
        let reveal_frame = pull_layer_frame(&mut self.reveal, playback_time);

        let mask_rgba = match (&base_frame, &reveal_frame) {
            (Some(base), Some(reveal)) => Some(self.mask_engine.generate_mask(base, reveal)),
            _ => None,
        };

        LayerUpdate {
            base_frame,
            reveal_frame,
            mask_rgba,
            mask_size: (self.mask_engine.width, self.mask_engine.height),
            xray_background_color: self.xray_background_color,
        }
    }

    fn build_layer(name: &str, cfg: &LayerConfig, width: u32, height: u32) -> Result<Layer> {
        let layer_type = match cfg.source.r#type {
            LayerSourceType::StaticImage => LayerType::StaticImage,
            LayerSourceType::Video => LayerType::Video,
            LayerSourceType::Gif => LayerType::Gif,
        };

        let mut layer = Layer {
            name: name.to_string(),
            layer_type,
            opacity: cfg.opacity,
            blend_mode: cfg.blend_mode,
            texture_size: (width, height),
            decoder: None,
            current_frame: None,
        };

        match layer_type {
            LayerType::StaticImage => {
                let frame = decode_static_image(Path::new(&cfg.source.path), width, height)
                    .with_context(|| format!("Failed loading static layer {name}"))?;
                layer.current_frame = Some(frame);
            }
            LayerType::Video | LayerType::Gif => {
                let decoder = VideoDecoder::new(
                    &cfg.source.path,
                    VideoDecoderConfig {
                        target_width: width,
                        target_height: height,
                        looping: cfg.source.looping,
                        max_buffered_frames: cfg.max_buffered_frames,
                        frame_cache_size: cfg.frame_cache_size,
                    },
                )
                .with_context(|| format!("Failed creating decoder for layer {name}"))?;
                layer.decoder = Some(decoder);
            }
        }

        Ok(layer)
    }

    pub fn inferred_source_paths(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let base = self
            .base
            .decoder
            .as_ref()
            .map(|d| d.source_path().to_path_buf());
        let reveal = self
            .reveal
            .decoder
            .as_ref()
            .map(|d| d.source_path().to_path_buf());
        (base, reveal)
    }
}

fn pull_layer_frame(layer: &mut Layer, playback_time: f64) -> Option<VideoFrame> {
    match layer.layer_type {
        LayerType::StaticImage => layer.current_frame.clone(),
        LayerType::Video | LayerType::Gif => {
            let Some(decoder) = layer.decoder.as_mut() else {
                return layer.current_frame.clone();
            };

            if let Some(duration) = decoder.stream_duration_seconds() {
                let local_time = if duration > 0.0 {
                    playback_time.rem_euclid(duration)
                } else {
                    playback_time
                };
                if let Some(frame) = decoder.poll_latest_for_time(local_time) {
                    layer.current_frame = Some(frame.clone());
                    return Some(frame);
                }
            }

            let frame = decoder.poll_latest_for_time(playback_time);
            if let Some(ref f) = frame {
                layer.current_frame = Some(f.clone());
            }
            frame.or_else(|| layer.current_frame.clone())
        }
    }
}

fn decode_static_image(path: &Path, width: u32, height: u32) -> Result<VideoFrame> {
    let img = image::open(path).with_context(|| format!("Cannot open image {}", path.display()))?;
    let resized = img
        .resize_to_fill(width, height, image::imageops::FilterType::Lanczos3)
        .to_rgba8();

    Ok(VideoFrame {
        rgba: resized.into_raw(),
        width,
        height,
        pts_seconds: 0.0,
        duration_seconds: 1.0 / 60.0,
        sequence: 0,
    })
}

fn luma(r: u8, g: u8, b: u8) -> f32 {
    (0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32)) / 255.0
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 >= edge1 {
        return if x >= edge0 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
