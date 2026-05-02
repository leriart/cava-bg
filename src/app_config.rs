#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default, alias = "bars")]
    pub audio: AudioConfig,
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub smoothing: SmoothingConfig,
    #[serde(default)]
    pub hidden_image: Option<HiddenImageConfig>,
    #[serde(default)]
    pub layers: Option<LayersConfig>,
    #[serde(default)]
    pub parallax: ParallaxConfig,
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    #[serde(default)]
    pub xray_mask: XrayMaskConfig,
    #[serde(default)]
    pub xray: XRayConfig,
    #[serde(default)]
    pub performance: PerformanceConfig,
    #[serde(default)]
    pub advanced: AdvancedConfig,
    #[serde(default)]
    pub global: Option<ConfigOverride>,
    #[serde(default)]
    pub output: BTreeMap<String, OutputOverrideConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ConfigOverride {
    #[serde(default)]
    pub general: Option<GeneralConfig>,
    #[serde(default, alias = "bars")]
    pub audio: Option<AudioConfig>,
    #[serde(default)]
    pub colors: Option<ColorsConfig>,
    #[serde(default)]
    pub display: Option<DisplayConfig>,
    #[serde(default)]
    pub smoothing: Option<SmoothingConfig>,
    #[serde(default)]
    pub hidden_image: Option<HiddenImageConfig>,
    #[serde(default)]
    pub layers: Option<LayersConfig>,
    #[serde(default)]
    pub parallax: Option<ParallaxConfig>,
    #[serde(default)]
    pub wallpaper: Option<WallpaperConfig>,
    #[serde(default)]
    pub xray_mask: Option<XrayMaskConfig>,
    #[serde(default)]
    pub xray: Option<XRayConfig>,
    #[serde(default)]
    pub performance: Option<PerformanceConfig>,
    #[serde(default)]
    pub advanced: Option<AdvancedConfig>,
}

impl ConfigOverride {
    fn normalize_compat_fields(&mut self) {
        if let Some(audio) = self.audio.as_mut() {
            audio._legacy_bar_gradient =
                audio._legacy_bar_gradient || audio._legacy_gradient.enabled;
            audio._legacy_glow_effect = audio._legacy_glow_effect || audio._legacy_glow.enabled;
            if audio._legacy_gradient_colors.is_empty() {
                audio._legacy_gradient_colors = audio._legacy_gradient.colors.clone();
            }
            if audio._legacy_gradient.colors.is_empty() {
                audio._legacy_gradient.colors = audio._legacy_gradient_colors.clone();
            }
            audio._legacy_gradient.enabled = audio._legacy_bar_gradient;
            audio._legacy_gradient.colors = audio._legacy_gradient_colors.clone();
            audio._legacy_gradient.direction = audio._legacy_gradient_direction;
            audio._legacy_glow.enabled = audio._legacy_glow_effect;
            audio._legacy_glow.intensity = audio._legacy_glow_intensity;
        }

        if let Some(colors) = self.colors.as_mut() {
            if colors.palette.is_empty() {
                let parsed = colors
                    .legacy_gradient_colors
                    .iter()
                    .filter_map(|(_, value)| parse_legacy_color(value))
                    .collect::<Vec<_>>();
                if !parsed.is_empty() {
                    colors.palette = parsed;
                }
            }
        }

        if let Some(parallax) = self.parallax.as_mut() {
            parallax.normalize_compat_fields();
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OutputOverrideConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub connector: Option<String>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(flatten)]
    pub config: ConfigOverride,
}

#[derive(Debug, Clone)]
pub struct OutputDescriptor {
    pub name: String,
    pub connector: Option<String>,
    pub index: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GeneralConfig {
    #[serde(default = "default_framerate")]
    pub framerate: u32,
    #[serde(default = "default_config_color")]
    pub background_color: ConfigColor,
    pub autosens: Option<bool>,
    pub sensitivity: Option<f32>,
    #[serde(default, alias = "preferred_output")]
    pub preferred_outputs: Vec<String>,
    #[serde(default)]
    pub dynamic_colors: bool,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: f32,
    #[serde(default)]
    pub disable_audio: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            framerate: default_framerate(),
            background_color: default_config_color(),
            autosens: None,
            sensitivity: None,
            preferred_outputs: Vec::new(),
            dynamic_colors: false,
            corner_radius: default_corner_radius(),
            disable_audio: false,
        }
    }
}

fn default_framerate() -> u32 {
    60
}

fn default_corner_radius() -> f32 {
    0.0
}

fn default_config_color() -> ConfigColor {
    ConfigColor::Complex(HexColorConfig {
        hex: "#000000".to_string(),
        alpha: Some(0.0),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AudioConfig {
    #[serde(default = "default_bar_amount", alias = "amount")]
    pub bar_count: u32,
    #[serde(default = "default_bar_width")]
    pub bar_width: f32,
    #[serde(default = "default_bar_spacing")]
    pub bar_spacing: f32,
    #[serde(default = "default_bar_gap")]
    pub gap: f32,
    #[serde(default = "default_bar_alpha")]
    pub bar_alpha: f32,
    #[serde(default = "default_height_scale")]
    pub height_scale: f32,
    #[serde(default = "default_visualizer_smoothing")]
    pub smoothing: f32,
    #[serde(default = "default_bar_color")]
    pub bar_color: ConfigColor,
    #[serde(default = "default_max_bar_height")]
    pub max_bar_height: f32,
    #[serde(default = "default_min_bar_height")]
    pub min_bar_height: f32,
    #[serde(default, skip_serializing)]
    pub mirror_bars: bool,
    #[serde(default)]
    pub bar_shape: BarShape,
    #[serde(default = "default_corner_radius_px")]
    pub corner_radius: f32,
    #[serde(default = "default_corner_segments")]
    pub corner_segments: u32,
    #[serde(default, alias = "bar_gradient")]
    pub _legacy_bar_gradient: bool,
    #[serde(default, alias = "gradient", skip_serializing)]
    pub _legacy_gradient: LegacyGradientConfig,
    #[serde(default, alias = "glow", skip_serializing)]
    pub _legacy_glow: LegacyGlowConfig,
    #[serde(default, alias = "gradient_colors", skip_serializing)]
    pub _legacy_gradient_colors: Vec<[f32; 4]>,
    #[serde(default, alias = "gradient_direction", skip_serializing)]
    pub _legacy_gradient_direction: GradientDirection,
    #[serde(default, alias = "glow_effect", skip_serializing)]
    pub _legacy_glow_effect: bool,
    #[serde(
        default = "default_glow_intensity",
        alias = "glow_intensity",
        skip_serializing
    )]
    pub _legacy_glow_intensity: f32,
    #[serde(default)]
    pub extract_colors_from_wallpaper: bool,
    #[serde(default)]
    pub color_extraction_mode: ColorExtractionMode,
    #[serde(default)]
    pub visualization_mode: VisualizationMode,
    #[serde(default = "default_polygon_sides")]
    pub polygon_sides: u32,
    #[serde(default = "default_true")]
    pub show_visualizer: bool, // Visualization mode parameters
    #[serde(default = "default_radial_inner_radius")]
    pub radial_inner_radius: f32,
    #[serde(default = "default_radial_sweep_angle")]
    pub radial_sweep_angle: f32,
    #[serde(default = "default_waveform_line_width")]
    pub waveform_line_width: f32,
    #[serde(default = "default_waveform_smoothness")]
    pub waveform_smoothness: f32,
    #[serde(default = "default_block_size")]
    pub block_size: f32,
    #[serde(default = "default_block_spacing")]
    pub block_spacing: f32,
    /// Number of turns for Spiral visualization mode (1.0 = single revolution).
    #[serde(default = "default_spiral_turns")]
    pub spiral_turns: f32,
    /// Gap between mirrored halves in MirrorBars mode (0..1, fraction of the
    /// vertical extent reserved as a horizontal gutter at the center line).
    #[serde(default = "default_mirror_gap")]
    pub mirror_gap: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            bar_count: default_bar_amount(),
            bar_width: default_bar_width(),
            bar_spacing: default_bar_spacing(),
            gap: default_bar_gap(),
            bar_alpha: default_bar_alpha(),
            height_scale: default_height_scale(),
            smoothing: default_visualizer_smoothing(),
            bar_color: default_bar_color(),
            max_bar_height: default_max_bar_height(),
            min_bar_height: default_min_bar_height(),
            mirror_bars: false,
            bar_shape: BarShape::default(),
            corner_radius: default_corner_radius_px(),
            corner_segments: default_corner_segments(),
            _legacy_bar_gradient: false,
            _legacy_gradient: LegacyGradientConfig::default(),
            _legacy_glow: LegacyGlowConfig::default(),
            _legacy_gradient_colors: default_gradient_colors(),
            _legacy_gradient_direction: GradientDirection::BottomToTop,
            _legacy_glow_effect: false,
            _legacy_glow_intensity: default_glow_intensity(),
            extract_colors_from_wallpaper: false,
            color_extraction_mode: ColorExtractionMode::Dominant,
            visualization_mode: VisualizationMode::Bars,
            polygon_sides: default_polygon_sides(),
            show_visualizer: true,
            radial_inner_radius: default_radial_inner_radius(),
            radial_sweep_angle: default_radial_sweep_angle(),
            waveform_line_width: default_waveform_line_width(),
            waveform_smoothness: default_waveform_smoothness(),
            block_size: default_block_size(),
            block_spacing: default_block_spacing(),
            spiral_turns: default_spiral_turns(),
            mirror_gap: default_mirror_gap(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LegacyGradientConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gradient_colors")]
    pub colors: Vec<[f32; 4]>,
    #[serde(default)]
    pub direction: GradientDirection,
}

impl Default for LegacyGradientConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            colors: default_gradient_colors(),
            direction: GradientDirection::BottomToTop,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LegacyGlowConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_glow_intensity")]
    pub intensity: f32,
}

impl Default for LegacyGlowConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: default_glow_intensity(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientDirection {
    TopToBottom,
    #[default]
    BottomToTop,
    LeftToRight,
    RightToLeft,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorExtractionMode {
    #[default]
    Dominant,
    Vibrant,
    Palette,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualizationMode {
    #[default]
    Bars,
    Waveform,
    Blocks,
    /// Bars mirrored vertically: each bar extends symmetrically up and down
    /// from the horizontal center line. Ideal for music players / DJ visuals.
    MirrorBars,
    /// Bars hanging from the top edge instead of growing from the bottom.
    InvertedBars,
    /// Smooth thick line connecting the top of each bar — a classic
    /// "spectrum analyzer" curve.
    Spectrum,
    /// Continuous filled ring whose thickness is modulated by the bin
    /// amplitudes (a polar variant of Bars).
    Ring,
}

impl<'de> serde::Deserialize<'de> for VisualizationMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "Bars" | "bars" => Ok(VisualizationMode::Bars),
            "Radial" | "radial" | "Spiral" | "spiral" => {
                eprintln!("Note: Radial/Spiral modes are deprecated, falling back to Ring");
                Ok(VisualizationMode::Ring)
            }
            "Waveform" | "waveform" => Ok(VisualizationMode::Waveform),
            "Blocks" | "blocks" => Ok(VisualizationMode::Blocks),
            "MirrorBars" | "mirror" | "Mirror" | "mirror_bars" => Ok(VisualizationMode::MirrorBars),
            "InvertedBars" | "inverted" | "Inverted" | "inverted_bars" => {
                Ok(VisualizationMode::InvertedBars)
            }
            "Spectrum" | "spectrum" | "Line" | "line" => Ok(VisualizationMode::Spectrum),
            "Ring" | "ring" => Ok(VisualizationMode::Ring),
            _ => {
                eprintln!(
                    "Warning: unknown visualization_mode '{}', falling back to Bars",
                    s
                );
                Ok(VisualizationMode::Bars)
            }
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarShape {
    #[default]
    Rectangle,
    Circle,
    Triangle,
    Line,
}

impl<'de> serde::Deserialize<'de> for BarShape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "Rectangle" | "rectangle" => Ok(BarShape::Rectangle),
            "Circle" | "circle" => Ok(BarShape::Circle),
            "Triangle" | "triangle" => Ok(BarShape::Triangle),
            "Line" | "line" => Ok(BarShape::Line),
            _ => {
                eprintln!(
                    "Warning: unknown bar_shape '{}', falling back to Rectangle",
                    s
                );
                Ok(BarShape::Rectangle)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BarShapeConfig {
    #[serde(default)]
    pub shape: BarShape,
    #[serde(default = "default_corner_radius_px")]
    pub corner_radius: f32,
    #[serde(default = "default_corner_segments")]
    pub corner_segments: u32,
}

impl Default for BarShapeConfig {
    fn default() -> Self {
        Self {
            shape: BarShape::default(),
            corner_radius: default_corner_radius_px(),
            corner_segments: default_corner_segments(),
        }
    }
}

fn default_corner_radius_px() -> f32 {
    6.0
}
fn default_corner_segments() -> u32 {
    8
}

fn default_polygon_sides() -> u32 {
    6
}

fn default_radial_inner_radius() -> f32 {
    30.0
}
fn default_radial_sweep_angle() -> f32 {
    360.0
}
fn default_waveform_line_width() -> f32 {
    2.0
}
fn default_waveform_smoothness() -> f32 {
    0.5
}
fn default_block_size() -> f32 {
    10.0
}
fn default_block_spacing() -> f32 {
    2.0
}
fn default_spiral_turns() -> f32 {
    2.0
}
fn default_mirror_gap() -> f32 {
    0.04
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ColorsConfig {
    #[serde(default)]
    pub extract_from_wallpaper: bool,
    #[serde(default)]
    pub extraction_mode: ColorExtractionMode,
    #[serde(default = "default_palette_colors")]
    pub palette: Vec<[f32; 4]>,
    #[serde(default)]
    pub gradient_direction: GradientDirection,
    #[serde(default = "default_true")]
    pub use_gradient: bool,
    #[serde(default, flatten)]
    pub legacy_gradient_colors: HashMap<String, String>,
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            extract_from_wallpaper: false,
            extraction_mode: ColorExtractionMode::Dominant,
            palette: default_palette_colors(),
            gradient_direction: GradientDirection::BottomToTop,
            use_gradient: true,
            legacy_gradient_colors: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DisplayConfig {
    #[serde(default)]
    pub position: Position,
    #[serde(default = "default_anchor_true")]
    pub anchor_top: bool,
    #[serde(default = "default_anchor_true")]
    pub anchor_bottom: bool,
    #[serde(default = "default_anchor_true")]
    pub anchor_left: bool,
    #[serde(default = "default_anchor_true")]
    pub anchor_right: bool,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub margin_top: u32,
    #[serde(default)]
    pub margin_bottom: u32,
    #[serde(default)]
    pub margin_left: u32,
    #[serde(default)]
    pub margin_right: u32,
    #[serde(default)]
    pub layer: LayerChoice,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub scale_with_resolution: bool,
    #[serde(default, alias = "margin", skip_serializing)]
    pub legacy_margin: Option<f32>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            position: Position::Fill,
            anchor_top: true,
            anchor_bottom: true,
            anchor_left: true,
            anchor_right: true,
            width: 0,
            height: 0,
            margin_top: 0,
            margin_bottom: 0,
            margin_left: 0,
            margin_right: 0,
            layer: LayerChoice::Bottom,
            opacity: default_opacity(),
            scale_with_resolution: false,
            legacy_margin: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Default)]
pub enum Position {
    #[default]
    Fill,
    Center,
    Top,
    Bottom,
    Left,
    Right,
    #[serde(
        alias = "TopLeft",
        alias = "TopRight",
        alias = "BottomLeft",
        alias = "BottomRight"
    )]
    Custom,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Default)]
pub enum LayerChoice {
    Background,
    #[default]
    Bottom,
    Top,
    Overlay,
}

fn default_bar_amount() -> u32 {
    76
}
fn default_bar_width() -> f32 {
    5.0
}
fn default_bar_spacing() -> f32 {
    2.0
}
fn default_bar_gap() -> f32 {
    0.1
}
fn default_bar_alpha() -> f32 {
    1.0
}
fn default_height_scale() -> f32 {
    1.0
}
fn default_visualizer_smoothing() -> f32 {
    0.8
}
fn default_bar_color() -> ConfigColor {
    ConfigColor::Complex(HexColorConfig {
        hex: "#ff00ff".to_string(),
        alpha: Some(1.0),
    })
}
fn default_max_bar_height() -> f32 {
    100.0
}
fn default_min_bar_height() -> f32 {
    0.0
}
fn default_opacity() -> f32 {
    1.0
}
fn default_glow_intensity() -> f32 {
    0.5
}
fn default_gradient_colors() -> Vec<[f32; 4]> {
    vec![[1.0, 0.0, 1.0, 1.0], [0.0, 1.0, 1.0, 1.0]]
}

fn default_palette_colors() -> Vec<[f32; 4]> {
    vec![
        [0.58, 0.89, 0.84, 1.0],
        [0.45, 0.78, 0.93, 1.0],
        [0.80, 0.65, 0.97, 1.0],
        [0.96, 0.76, 0.90, 1.0],
    ]
}

fn default_anchor_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SmoothingConfig {
    pub monstercat: Option<f32>,
    pub waves: Option<i32>,
    pub noise_reduction: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ConfigColor {
    Simple(String),
    Complex(HexColorConfig),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HexColorConfig {
    pub hex: String,
    pub alpha: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CavaConfig {
    pub general: CavaGeneralConfig,
    pub smoothing: CavaSmoothingConfig,
    pub output: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CavaGeneralConfig {
    pub framerate: u32,
    pub bars: u32,
    pub autosens: Option<bool>,
    pub sensitivity: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CavaSmoothingConfig {
    pub monstercat: Option<f32>,
    pub waves: Option<i32>,
    pub noise_reduction: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HiddenImageConfig {
    #[serde(default)]
    pub use_wallpaper: bool,
    pub path: Option<String>,
    #[serde(default)]
    pub effect: HiddenImageEffect,
    #[serde(default)]
    pub blend_mode: BlendMode,
    pub wallpapers_dir: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayersConfig {
    pub base: LayerConfig,
    pub reveal: LayerConfig,
    #[serde(default)]
    pub sync: LayerSyncConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerConfig {
    #[serde(default = "default_layer_enabled")]
    pub enabled: bool,
    pub source: LayerSourceConfig,
    #[serde(default = "default_layer_fit")]
    pub fit: String,
    #[serde(default = "default_layer_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub blend_mode: BlendMode,
    #[serde(default = "default_max_buffered_frames")]
    pub max_buffered_frames: usize,
    #[serde(default = "default_frame_cache_size")]
    pub frame_cache_size: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerSourceConfig {
    #[serde(rename = "type")]
    pub r#type: LayerSourceType,
    pub path: String,
    #[serde(default = "default_looping")]
    pub looping: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerSourceType {
    #[serde(rename = "static")]
    StaticImage,
    #[serde(rename = "video")]
    Video,
    #[serde(rename = "gif")]
    Gif,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerSyncConfig {
    #[serde(default = "default_sync_with_wallpaper")]
    pub sync_with_wallpaper: bool,
    #[serde(default = "default_fingerprint_search_window")]
    pub fingerprint_search_window: usize,
    #[serde(default = "default_fingerprint_min_confidence")]
    pub fingerprint_min_confidence: f32,
}

impl Default for LayerSyncConfig {
    fn default() -> Self {
        Self {
            sync_with_wallpaper: default_sync_with_wallpaper(),
            fingerprint_search_window: default_fingerprint_search_window(),
            fingerprint_min_confidence: default_fingerprint_min_confidence(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WallpaperConfig {
    #[serde(default = "default_auto_detect_wallpaper")]
    pub auto_detect_wallpaper: bool,
    #[serde(default)]
    pub xray_layers_dir: Option<PathBuf>,
    #[serde(default)]
    pub wallpapers_dir: Option<PathBuf>,
    #[serde(default = "default_sync_interval_seconds")]
    pub sync_interval_seconds: f64,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            auto_detect_wallpaper: default_auto_detect_wallpaper(),
            xray_layers_dir: None,
            wallpapers_dir: None,
            sync_interval_seconds: default_sync_interval_seconds(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ParallaxMode,
    #[serde(default)]
    pub enable_3d_depth: bool,
    #[serde(default)]
    pub mouse: ParallaxMouseConfig,
    #[serde(default)]
    pub performance: ParallaxPerformanceConfig,
    #[serde(default)]
    pub preset: Option<ParallaxPreset>,
    #[serde(default)]
    pub layers: Vec<ParallaxLayerConfig>,
    #[serde(default = "default_true")]
    pub show_visualizer: bool,
    #[serde(default)]
    pub visualizer_as_parallax_layer: bool,
    pub visualizer_layer_index: usize,
    #[serde(default)]
    pub profiles_dir: Option<PathBuf>,
    #[serde(default)]
    pub profile_source: ProfileSource,
    #[serde(default)]
    pub active_profile: Option<String>,
}

impl Default for ParallaxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ParallaxMode::Hybrid,
            enable_3d_depth: false,
            mouse: ParallaxMouseConfig::default(),
            performance: ParallaxPerformanceConfig::default(),
            preset: None,
            layers: Vec::new(),
            show_visualizer: true,
            visualizer_as_parallax_layer: false,
            visualizer_layer_index: 0,
            profiles_dir: None,
            profile_source: ProfileSource::FromWallpaper,
            active_profile: None,
        }
    }
}

impl ParallaxConfig {
    pub fn normalize_compat_fields(&mut self) {
        for layer in &mut self.layers {
            if layer.source.trim().starts_with("effect:") && layer.effect.is_none() {
                let mut effect_cfg = ParallaxEffectConfig::default();
                effect_cfg.enabled = true;
                let tag = layer
                    .source
                    .trim()
                    .trim_start_matches("effect:")
                    .to_ascii_lowercase();
                effect_cfg.effect_type = match tag.as_str() {
                    "cava-wave" | "wave" => ParallaxEffectType::CavaWave,
                    "cava-radial" | "radial" => ParallaxEffectType::CavaRadial,
                    _ => ParallaxEffectType::CavaBars,
                };
                layer.effect = Some(effect_cfg);
            }

            if layer.layer_type.is_none() && !layer.source.trim().is_empty() {
                layer.layer_type = Some(
                    match layer
                        .source
                        .rsplit_once('.')
                        .map(|(_, ext)| ext.to_ascii_lowercase())
                        .unwrap_or_default()
                        .as_str()
                    {
                        "gif" => LayerSourceType::Gif,
                        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" | "flv" | "wmv" => {
                            LayerSourceType::Video
                        }
                        _ => LayerSourceType::StaticImage,
                    },
                );
            }

            if layer.react_to_audio && !layer.audio.enabled {
                layer.audio.enabled = true;
                layer.audio.amplitude_sensitivity = layer.audio_reaction_intensity;
            }

            if layer.react_to_mouse && !layer.mouse.enabled {
                layer.mouse.enabled = true;
                layer.mouse.sensitivity = layer.parallax_speed.max(0.01);
                layer.mouse.max_offset = [
                    32.0 * layer.mouse_depth_factor.max(0.1),
                    32.0 * layer.mouse_depth_factor.max(0.1),
                ];
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParallaxMode {
    #[default]
    AudioReactive,
    MouseReactive,
    Animated,
    Hybrid,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileSource {
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "wallpaper")]
    #[default]
    FromWallpaper,
}

/// A saved parallax profile — a named collection of layers with config.
/// Stored as {profiles_dir}/{name}/parallax.toml
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxProfile {
    #[serde(default)]
    pub name: String,
    /// Original image used to create this parallax (optional)
    #[serde(default)]
    pub source_image: Option<String>,
    /// Name of the wallpaper this profile was created from (for auto_match)
    #[serde(default)]
    pub wallpaper_name: Option<String>,
    /// Ordered list of layer image filenames within this profile's directory
    #[serde(default)]
    pub layers: Vec<String>,
    /// Per-layer overrides (keyed by filename without extension)
    #[serde(default)]
    pub layer: BTreeMap<String, ParallaxLayerConfig>,
}

impl ParallaxProfile {
    fn layer_path(&self, profile_dir: &Path, layer_name: &str) -> Option<PathBuf> {
        let p = profile_dir.join(layer_name);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    /// Resolve the full path for a layer (relative to profile_dir)
    pub fn resolve_layer(&self, profile_dir: &Path, layer_file: &str) -> PathBuf {
        let exact = profile_dir.join(layer_file);
        if exact.exists() {
            return exact;
        }
        // Fallback: find first image file in the profile directory
        if let Ok(entries) = std::fs::read_dir(profile_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if matches!(
                            ext.to_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "gif"
                        ) {
                            return path;
                        }
                    }
                }
            }
        }
        exact
    }

    /// Get layer config for a given layer filename, merged with defaults
    pub fn layer_config(&self, layer_file: &str) -> ParallaxLayerConfig {
        // Use file stem as key (without extension)
        let key = std::path::Path::new(layer_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(layer_file)
            .to_string();
        self.layer.get(&key).cloned().unwrap_or_default()
    }

    pub fn update_layer_config(&mut self, layer_file: &str, config: ParallaxLayerConfig) {
        let key = std::path::Path::new(layer_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(layer_file)
            .to_string();
        self.layer.insert(key, config);
    }

    /// Discover profiles in a directory
    pub fn discover_profiles(profiles_dir: &Path) -> Vec<String> {
        let mut profiles = Vec::new();
        if let Ok(entries) = std::fs::read_dir(profiles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("parallax.toml").exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        profiles.push(name.to_string());
                    }
                }
            }
        }
        profiles.sort();
        profiles
    }

    /// Load a profile from its directory
    pub fn load(profiles_dir: &Path, name: &str) -> Result<Self, anyhow::Error> {
        let path = profiles_dir.join(name).join("parallax.toml");
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
        let mut profile: Self = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;
        profile.name = name.to_string();
        Ok(profile)
    }

    /// Save profile to its directory
    pub fn save(&self, profiles_dir: &Path) -> Result<(), anyhow::Error> {
        let dir = profiles_dir.join(&self.name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("parallax.toml");
        let content = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize: {}", e))?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Create a new profile from a source image
    pub fn create(profiles_dir: &Path, name: &str, source: &Path) -> Result<Self, anyhow::Error> {
        let dir = profiles_dir.join(name);
        std::fs::create_dir_all(&dir)?;
        // Copy source image as layer1.png
        let dest = dir.join("layer1.png");
        std::fs::copy(source, &dest)?;
        let profile = Self {
            name: name.to_string(),
            source_image: Some(source.to_string_lossy().to_string()),
            wallpaper_name: None,
            layers: vec!["layer1.png".to_string()],
            layer: BTreeMap::new(),
        };
        profile.save(profiles_dir)?;
        Ok(profile)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallaxPreset {
    SoftDepth,
    AudioPulse,
    Cinematic,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxMouseConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_parallax_mouse_sensitivity")]
    pub sensitivity: f32,
    #[serde(default = "default_parallax_mouse_range")]
    pub range: f32,
    #[serde(default = "default_true")]
    pub global_tracking: bool,
    #[serde(default = "default_true")]
    pub per_output_tracking: bool,
}

impl Default for ParallaxMouseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sensitivity: default_parallax_mouse_sensitivity(),
            range: default_parallax_mouse_range(),
            global_tracking: true,
            per_output_tracking: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxPerformanceConfig {
    #[serde(default)]
    pub disable_under_load: bool,
    #[serde(default = "default_parallax_frame_time_budget_ms")]
    pub frame_time_budget_ms: f32,
    #[serde(default = "default_true")]
    pub lazy_load_assets: bool,
    #[serde(default = "default_true")]
    pub pause_on_idle: bool,
}

impl Default for ParallaxPerformanceConfig {
    fn default() -> Self {
        Self {
            disable_under_load: false,
            frame_time_budget_ms: default_parallax_frame_time_budget_ms(),
            lazy_load_assets: true,
            pause_on_idle: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrequencyZone {
    #[default]
    FullSpectrum,
    Low,
    Mid,
    High,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioResponseCurve {
    Linear,
    #[default]
    Smooth,
    Exponential,
    Punchy,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxLayerAudioConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub frequency_zone: FrequencyZone,
    #[serde(default)]
    pub response_curve: AudioResponseCurve,
    #[serde(default = "default_audio_reaction_intensity")]
    pub amplitude_sensitivity: f32,
    #[serde(default = "default_audio_frequency_sensitivity")]
    pub frequency_sensitivity: f32,
    #[serde(default)]
    pub transform: LayerAudioTransformConfig,
}

impl Default for ParallaxLayerAudioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency_zone: FrequencyZone::FullSpectrum,
            response_curve: AudioResponseCurve::Smooth,
            amplitude_sensitivity: default_audio_reaction_intensity(),
            frequency_sensitivity: default_audio_frequency_sensitivity(),
            transform: LayerAudioTransformConfig::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerAudioTransformConfig {
    #[serde(default = "default_true")]
    pub shift: bool,
    #[serde(default)]
    pub scale: bool,
    #[serde(default)]
    pub rotate: bool,
    #[serde(default = "default_audio_shift_amount")]
    pub shift_amount: f32,
    #[serde(default = "default_audio_scale_amount")]
    pub scale_amount: f32,
    #[serde(default = "default_audio_rotation_amount")]
    pub rotation_amount: f32,
}

impl Default for LayerAudioTransformConfig {
    fn default() -> Self {
        Self {
            shift: true,
            scale: false,
            rotate: false,
            shift_amount: default_audio_shift_amount(),
            scale_amount: default_audio_scale_amount(),
            rotation_amount: default_audio_rotation_amount(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerMouseReactivityConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_parallax_mouse_sensitivity")]
    pub sensitivity: f32,
    #[serde(default = "default_layer_mouse_max_offset")]
    pub max_offset: [f32; 2],
}

impl Default for LayerMouseReactivityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sensitivity: default_parallax_mouse_sensitivity(),
            max_offset: default_layer_mouse_max_offset(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParallaxEffectType {
    #[default]
    CavaBars,
    CavaWave,
    CavaRadial,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxEffectConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub effect_type: ParallaxEffectType,
    #[serde(default = "default_effect_bars")]
    pub bars: u32,
    #[serde(default = "default_effect_tint")]
    pub tint: [f32; 4],
    #[serde(default)]
    pub gap: f32,
    #[serde(default)]
    pub height_scale: f32,
}

impl Default for ParallaxEffectConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            effect_type: ParallaxEffectType::CavaBars,
            bars: default_effect_bars(),
            tint: default_effect_tint(),
            gap: 0.0,
            height_scale: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallaxLayerConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub layer_type: Option<LayerSourceType>,
    #[serde(default)]
    pub effect: Option<ParallaxEffectConfig>,
    #[serde(default)]
    pub z_index: i32,
    #[serde(default = "default_parallax_depth")]
    pub depth: f32,
    #[serde(default)]
    pub opacity: f32,
    #[serde(default)]
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default)]
    pub parallax_speed: f32,
    #[serde(default)]
    pub audio: ParallaxLayerAudioConfig,
    #[serde(default)]
    pub mouse: LayerMouseReactivityConfig,
    #[serde(default)]
    pub react_to_audio: bool,
    #[serde(default = "default_audio_reaction_intensity")]
    pub audio_reaction_intensity: f32,
    #[serde(default)]
    pub react_to_mouse: bool,
    #[serde(default = "default_mouse_depth_factor")]
    pub mouse_depth_factor: f32,
    #[serde(default)]
    pub animation: Option<LayerAnimationConfig>,
    #[serde(default)]
    pub drop_shadow: Option<DropShadowConfig>,
}

impl Default for ParallaxLayerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            source: String::new(),
            layer_type: None,
            effect: None,
            z_index: 0,
            depth: default_parallax_depth(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            offset: [0.0, 0.0],
            parallax_speed: 0.5,
            audio: ParallaxLayerAudioConfig::default(),
            mouse: LayerMouseReactivityConfig::default(),
            react_to_audio: false,
            audio_reaction_intensity: default_audio_reaction_intensity(),
            react_to_mouse: true,
            mouse_depth_factor: default_mouse_depth_factor(),
            animation: None,
            drop_shadow: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DropShadowConfig {
    pub color: [f32; 4],
    pub offset: [f32; 2],
    pub blur_radius: f32,
    #[serde(default)]
    pub spread: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayerAnimationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "type", default)]
    pub animation_type: AnimationType,
    #[serde(default = "default_animation_speed")]
    pub speed: f32,
    #[serde(default = "default_animation_amplitude")]
    pub amplitude: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationType {
    #[default]
    Float,
    Rotate,
    Scale,
    Pulse,
    Wiggle,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerformanceConfig {
    #[serde(default = "default_vsync")]
    pub vsync: bool,
    #[serde(default)]
    pub multi_threaded_decode: bool,
    #[serde(default)]
    pub idle_mode: IdleModeConfig,
    #[serde(default)]
    pub video_decoder: VideoDecoderPerfConfig,
    #[serde(default)]
    pub xray: XrayPerformanceConfig,
    #[serde(default)]
    pub telemetry: PerformanceTelemetryConfig,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            vsync: default_vsync(),
            multi_threaded_decode: true,
            idle_mode: IdleModeConfig::default(),
            video_decoder: VideoDecoderPerfConfig::default(),
            xray: XrayPerformanceConfig::default(),
            telemetry: PerformanceTelemetryConfig::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IdleModeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_idle_audio_threshold")]
    pub audio_threshold: f32,
    #[serde(default = "default_idle_timeout_seconds")]
    pub timeout_seconds: f32,
    #[serde(default = "default_idle_fps")]
    pub idle_fps: u32,
    #[serde(default = "default_idle_exit_transition_ms")]
    pub exit_transition_ms: u32,
}

impl Default for IdleModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            audio_threshold: default_idle_audio_threshold(),
            timeout_seconds: default_idle_timeout_seconds(),
            idle_fps: default_idle_fps(),
            exit_transition_ms: default_idle_exit_transition_ms(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoDecoderPerfConfig {
    #[serde(default = "default_true")]
    pub lazy_init: bool,
    #[serde(default = "default_true")]
    pub auto_shutdown: bool,
    #[serde(default = "default_decoder_shutdown_seconds")]
    pub shutdown_after_seconds: f32,
    #[serde(default = "default_true")]
    pub pause_on_idle: bool,
    #[serde(default)]
    pub debug_telemetry: bool,
}

impl Default for VideoDecoderPerfConfig {
    fn default() -> Self {
        Self {
            lazy_init: true,
            auto_shutdown: true,
            shutdown_after_seconds: default_decoder_shutdown_seconds(),
            pause_on_idle: true,
            debug_telemetry: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskComputeMode {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XrayPerformanceConfig {
    #[serde(default = "default_xray_prescale_max_dimension")]
    pub prescale_max_dimension: u32,
    #[serde(default = "default_true")]
    pub generate_mipmaps: bool,
    #[serde(default)]
    pub mask_compute_mode: MaskComputeMode,
}

impl Default for XrayPerformanceConfig {
    fn default() -> Self {
        Self {
            prescale_max_dimension: default_xray_prescale_max_dimension(),
            generate_mipmaps: true,
            mask_compute_mode: MaskComputeMode::Auto,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerformanceTelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_perf_metrics_window")]
    pub metrics_window: usize,
    #[serde(default = "default_perf_log_interval_seconds")]
    pub log_interval_seconds: u64,
}

impl Default for PerformanceTelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            metrics_window: default_perf_metrics_window(),
            log_interval_seconds: default_perf_log_interval_seconds(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub enum HiddenImageEffect {
    #[default]
    None,
    Grayscale,
    Invert,
    Sepia,
    #[serde(rename = "palette")]
    Palette(PaletteType),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum PaletteType {
    Catppuccin,
    Nord,
    Gruvbox,
    Solarized,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub enum BlendMode {
    #[default]
    Reveal,
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XrayMaskConfig {
    #[serde(default = "default_xray_intensity", alias = "reveal_threshold")]
    pub intensity: f32,
    #[serde(default = "default_mask_gamma")]
    pub gamma: f32,
    #[serde(default = "default_mask_opacity")]
    pub opacity: f32,
    #[serde(default = "default_xray_blend_mode")]
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub xray_background_color: Option<[f32; 4]>,
    #[serde(default)]
    pub use_background: bool,
}

impl Default for XrayMaskConfig {
    fn default() -> Self {
        Self {
            intensity: default_xray_intensity(),
            gamma: default_mask_gamma(),
            opacity: default_mask_opacity(),
            blend_mode: default_xray_blend_mode(),
            xray_background_color: None,
            use_background: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdvancedConfig {
    #[serde(default)]
    pub verbose_logging: bool,
    #[serde(default = "default_frame_rate_limit")]
    pub frame_rate_limit: u32,
    #[serde(default = "default_layer_cache_size")]
    pub layer_cache_size: usize,
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            verbose_logging: false,
            frame_rate_limit: default_frame_rate_limit(),
            layer_cache_size: default_layer_cache_size(),
        }
    }
}

fn default_layer_enabled() -> bool {
    true
}
fn default_layer_fit() -> String {
    "cover".to_string()
}
fn default_layer_opacity() -> f32 {
    1.0
}
fn default_max_buffered_frames() -> usize {
    6
}
fn default_frame_cache_size() -> usize {
    120
}
fn default_looping() -> bool {
    true
}
fn default_sync_with_wallpaper() -> bool {
    true
}
fn default_fingerprint_search_window() -> usize {
    90
}
fn default_fingerprint_min_confidence() -> f32 {
    0.55
}
fn default_auto_detect_wallpaper() -> bool {
    true
}
fn default_sync_interval_seconds() -> f64 {
    10.0
}
fn default_xray_intensity() -> f32 {
    0.8
}
fn default_mask_gamma() -> f32 {
    1.2
}
fn default_mask_opacity() -> f32 {
    1.0
}
fn default_xray_blend_mode() -> BlendMode {
    BlendMode::Normal
}
fn default_frame_rate_limit() -> u32 {
    60
}
fn default_layer_cache_size() -> usize {
    5
}
fn default_vsync() -> bool {
    true
}
fn default_idle_audio_threshold() -> f32 {
    0.015
}
fn default_idle_timeout_seconds() -> f32 {
    5.0
}
fn default_idle_fps() -> u32 {
    10
}
fn default_idle_exit_transition_ms() -> u32 {
    250
}
fn default_decoder_shutdown_seconds() -> f32 {
    20.0
}
fn default_xray_prescale_max_dimension() -> u32 {
    2048
}
fn default_perf_metrics_window() -> usize {
    240
}
fn default_perf_log_interval_seconds() -> u64 {
    5
}
fn default_audio_reaction_intensity() -> f32 {
    0.5
}
fn default_audio_frequency_sensitivity() -> f32 {
    0.5
}
fn default_mouse_depth_factor() -> f32 {
    0.8
}
fn default_parallax_mouse_sensitivity() -> f32 {
    0.35
}
fn default_parallax_mouse_range() -> f32 {
    1.0
}
fn default_layer_mouse_max_offset() -> [f32; 2] {
    [32.0, 32.0]
}
fn default_parallax_depth() -> f32 {
    0.5
}
fn default_parallax_frame_time_budget_ms() -> f32 {
    18.0
}
fn default_audio_shift_amount() -> f32 {
    28.0
}
fn default_audio_scale_amount() -> f32 {
    0.08
}
fn default_audio_rotation_amount() -> f32 {
    6.0
}
fn default_animation_speed() -> f32 {
    0.5
}
fn default_animation_amplitude() -> f32 {
    10.0
}
fn default_effect_bars() -> u32 {
    48
}
fn default_effect_tint() -> [f32; 4] {
    [0.75, 0.85, 1.0, 0.95]
}

pub fn color_from_hex(hex: String, a: f32) -> [f32; 4] {
    let r = u8::from_str_radix(&hex[1..3], 16).unwrap() as f32 / 255f32;
    let g = u8::from_str_radix(&hex[3..5], 16).unwrap() as f32 / 255f32;
    let b = u8::from_str_radix(&hex[5..7], 16).unwrap() as f32 / 255f32;
    [r, g, b, a]
}

pub fn array_from_config_color(color: ConfigColor) -> [f32; 4] {
    match color {
        ConfigColor::Simple(hex) => color_from_hex(hex.to_string(), 1.0),
        ConfigColor::Complex(color) => {
            color_from_hex(color.hex.to_string(), color.alpha.unwrap_or(1.0))
        }
    }
}

fn parse_legacy_color(hex: &str) -> Option<[f32; 4]> {
    let value = hex.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&value[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&value[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&value[4..6], 16).ok()? as f32 / 255.0;
    Some([r, g, b, 1.0])
}

pub fn config_color_from_rgba(color: [f32; 4]) -> ConfigColor {
    let r = (color[0].clamp(0.0, 1.0) * 255.0) as u8;
    let g = (color[1].clamp(0.0, 1.0) * 255.0) as u8;
    let b = (color[2].clamp(0.0, 1.0) * 255.0) as u8;
    ConfigColor::Complex(HexColorConfig {
        hex: format!("#{:02x}{:02x}{:02x}", r, g, b),
        alpha: Some(color[3].clamp(0.0, 1.0)),
    })
}

fn wildcard_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }

    let p = pattern.as_bytes();
    let t = text.as_bytes();

    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(star) = star_idx {
            pi = star + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }

    pi == p.len()
}

fn pattern_specificity(pattern: &str) -> i32 {
    pattern
        .chars()
        .map(|c| match c {
            '*' => 0,
            '?' => 1,
            _ => 3,
        })
        .sum()
}

fn score_pattern(pattern: &str, value: &str) -> Option<i32> {
    if wildcard_matches(pattern, value) {
        Some(pattern_specificity(pattern))
    } else {
        None
    }
}

fn match_output_override(
    key: &str,
    cfg: &OutputOverrideConfig,
    descriptor: &OutputDescriptor,
) -> Option<i32> {
    let mut score = score_pattern(key, &descriptor.name)?;

    if let Some(name_pattern) = &cfg.name {
        score += 100 + score_pattern(name_pattern, &descriptor.name)?;
    }

    if let Some(connector_pattern) = &cfg.connector {
        let connector = descriptor.connector.as_deref()?;
        score += 70 + score_pattern(connector_pattern, connector)?;
    }

    if let Some(expected_index) = cfg.index {
        let actual_index = descriptor.index?;
        if expected_index != actual_index {
            return None;
        }
        score += 40;
    }

    if key.eq_ignore_ascii_case(&descriptor.name) {
        score += 1000;
    }

    Some(score)
}

impl Config {
    pub fn normalize_compat_fields(&mut self) {
        self.normalize_section_compat_fields();

        if let Some(global) = self.global.as_mut() {
            global.normalize_compat_fields();
        }
        for override_cfg in self.output.values_mut() {
            override_cfg.config.normalize_compat_fields();
        }
    }

    fn normalize_section_compat_fields(&mut self) {
        self.audio._legacy_bar_gradient =
            self.audio._legacy_bar_gradient || self.audio._legacy_gradient.enabled;
        self.audio._legacy_glow_effect =
            self.audio._legacy_glow_effect || self.audio._legacy_glow.enabled;

        if self.audio._legacy_gradient_colors.is_empty() {
            self.audio._legacy_gradient_colors = self.audio._legacy_gradient.colors.clone();
        }
        if self.audio._legacy_gradient.colors.is_empty() {
            self.audio._legacy_gradient.colors = self.audio._legacy_gradient_colors.clone();
        }

        self.audio._legacy_gradient.enabled = self.audio._legacy_bar_gradient;
        self.audio._legacy_gradient.colors = self.audio._legacy_gradient_colors.clone();
        self.audio._legacy_gradient.direction = self.audio._legacy_gradient_direction;

        self.audio._legacy_glow.enabled = self.audio._legacy_glow_effect;
        self.audio._legacy_glow.intensity = self.audio._legacy_glow_intensity;

        if let Some(margin) = self.display.legacy_margin {
            let margin_u32 = margin.max(0.0).round() as u32;
            if self.display.margin_top == 0
                && self.display.margin_bottom == 0
                && self.display.margin_left == 0
                && self.display.margin_right == 0
            {
                self.display.margin_top = margin_u32;
                self.display.margin_bottom = margin_u32;
                self.display.margin_left = margin_u32;
                self.display.margin_right = margin_u32;
            }
        }

        if self.colors.palette.is_empty() {
            let mut parsed = self
                .colors
                .legacy_gradient_colors
                .iter()
                .filter_map(|(_, value)| parse_legacy_color(value))
                .collect::<Vec<_>>();
            if parsed.is_empty() {
                parsed = default_palette_colors();
            }
            self.colors.palette = parsed;
        }

        self.parallax.normalize_compat_fields();
    }

    pub fn resolve_for_output(&self, descriptor: &OutputDescriptor) -> Option<Config> {
        let mut resolved = self.clone();
        resolved.output = BTreeMap::new();

        if let Some(global_override) = &self.global {
            resolved.apply_override(global_override);
        }

        if let Some((_, override_cfg)) = self.find_best_output_override(descriptor) {
            if override_cfg.enabled == Some(false) {
                return None;
            }
            resolved.apply_override(&override_cfg.config);
        }

        resolved.global = None;
        resolved.normalize_section_compat_fields();
        Some(resolved)
    }

    fn apply_override(&mut self, override_cfg: &ConfigOverride) {
        if let Some(value) = &override_cfg.general {
            self.general = value.clone();
        }
        if let Some(value) = &override_cfg.audio {
            self.audio = value.clone();
        }
        if let Some(value) = &override_cfg.colors {
            self.colors = value.clone();
        }
        if let Some(value) = &override_cfg.display {
            self.display = value.clone();
        }
        if let Some(value) = &override_cfg.smoothing {
            self.smoothing = value.clone();
        }
        if let Some(value) = &override_cfg.hidden_image {
            self.hidden_image = Some(value.clone());
        }
        if let Some(value) = &override_cfg.layers {
            self.layers = Some(value.clone());
        }
        if let Some(value) = &override_cfg.parallax {
            self.parallax = value.clone();
        }
        if let Some(value) = &override_cfg.wallpaper {
            self.wallpaper = value.clone();
        }
        if let Some(value) = &override_cfg.xray_mask {
            self.xray_mask = value.clone();
        }
        if let Some(value) = &override_cfg.xray {
            self.xray = value.clone();
        }
        if let Some(value) = &override_cfg.performance {
            self.performance = value.clone();
        }
        if let Some(value) = &override_cfg.advanced {
            self.advanced = value.clone();
        }
    }

    fn find_best_output_override(
        &self,
        descriptor: &OutputDescriptor,
    ) -> Option<(&String, &OutputOverrideConfig)> {
        self.output
            .iter()
            .filter_map(|(key, cfg)| {
                let score = match_output_override(key, cfg, descriptor)?;
                Some((score, key, cfg))
            })
            .max_by_key(|(score, _, _)| *score)
            .map(|(_, key, cfg)| (key, cfg))
    }

    pub fn configured_output_keys(&self) -> Vec<String> {
        self.output.keys().cloned().collect()
    }

    pub fn effective_layers_with_legacy_fallback(&self) -> Option<LayersConfig> {
        if let Some(layers) = &self.layers {
            return Some(layers.clone());
        }

        let hidden = self.hidden_image.as_ref()?;
        let path = hidden.path.clone()?;
        Some(LayersConfig {
            base: LayerConfig {
                enabled: true,
                source: LayerSourceConfig {
                    r#type: LayerSourceType::StaticImage,
                    path,
                    looping: true,
                },
                fit: "cover".to_string(),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                max_buffered_frames: default_max_buffered_frames(),
                frame_cache_size: default_frame_cache_size(),
            },
            reveal: LayerConfig {
                enabled: true,
                source: LayerSourceConfig {
                    r#type: LayerSourceType::StaticImage,
                    path: hidden.path.clone().unwrap_or_default(),
                    looping: true,
                },
                fit: "cover".to_string(),
                opacity: 1.0,
                blend_mode: hidden.blend_mode,
                max_buffered_frames: default_max_buffered_frames(),
                frame_cache_size: default_frame_cache_size(),
            },
            sync: LayerSyncConfig::default(),
        })
    }

    pub fn manual_layers_specified(&self) -> bool {
        self.layers
            .as_ref()
            .map(|layers| {
                !layers.base.source.path.trim().is_empty()
                    || !layers.reveal.source.path.trim().is_empty()
            })
            .unwrap_or(false)
    }

    pub fn auto_wallpaper_enabled(&self) -> bool {
        self.wallpaper.auto_detect_wallpaper && !self.manual_layers_specified()
    }

    pub fn resolve_xray_dir(&self) -> Option<PathBuf> {
        self.wallpaper
            .xray_layers_dir
            .as_ref()
            .map(expand_tilde_path)
            .or_else(|| dirs::home_dir().map(|h| h.join(".config/cava-bg/Xray")))
    }

    pub fn resolve_wallpapers_dir(&self) -> Option<PathBuf> {
        self.wallpaper
            .wallpapers_dir
            .as_ref()
            .map(expand_tilde_path)
            .or_else(|| dirs::home_dir().map(|h| h.join(".config/cava-bg/wallpapers")))
    }
}

// ============================================================================
// XRayConfig - Optional X-Ray effect with animation (OFF by default)
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XRayConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_xray_intensity")]
    pub intensity: f32,
    #[serde(default)]
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub base_layer_path: String,
    #[serde(default)]
    pub reveal_layer_path: String,
    #[serde(default = "default_true")]
    pub auto_detect: bool,
    #[serde(default)]
    pub use_background_color: bool,
    #[serde(default = "default_bg_color")]
    pub background_color: [f32; 4],
    // Xray images directory (counterpart hidden images)
    #[serde(default)]
    pub images_dir: Option<String>,
    // Animation
    #[serde(default)]
    pub animation_enabled: bool,
    #[serde(default)]
    pub animation_type: XRayAnimationType,
    #[serde(default = "default_anim_speed")]
    pub animation_speed: f32,
    #[serde(default = "default_audio_sens")]
    pub audio_sensitivity: f32,
    #[serde(default = "default_true")]
    pub auto_detect_wallpaper_fps: bool,
}

impl Default for XRayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: default_xray_intensity(),
            blend_mode: BlendMode::Normal,
            base_layer_path: String::new(),
            reveal_layer_path: String::new(),
            auto_detect: true,
            use_background_color: false,
            background_color: default_bg_color(),
            images_dir: None,
            animation_enabled: false,
            animation_type: XRayAnimationType::default(),
            animation_speed: default_anim_speed(),
            audio_sensitivity: default_audio_sens(),
            auto_detect_wallpaper_fps: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XRayAnimationType {
    #[default]
    None,
    Fade,
    Pulse,
    WaveReveal,
    AudioSync,
    WallpaperSync,
}

fn default_true() -> bool {
    true
}
fn default_bg_color() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn default_anim_speed() -> f32 {
    1.0
}
fn default_audio_sens() -> f32 {
    1.0
}

fn expand_tilde_path(path: &PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.clone()
}
