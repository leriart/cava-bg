use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub general: GeneralConfig,
    pub bars: BarConfig,
    pub colors: HashMap<String, ConfigColor>,
    pub smoothing: SmoothingConfig,
    #[serde(default)]
    pub hidden_image: Option<HiddenImageConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GeneralConfig {
    pub framerate: u32,
    pub background_color: ConfigColor,
    pub autosens: Option<bool>,
    pub sensitivity: Option<f32>,
    pub preferred_output: Option<String>,
    #[serde(default)]
    pub dynamic_colors: bool,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: f32,
}

fn default_corner_radius() -> f32 { 0.0 }

#[derive(Serialize, Deserialize, Debug)]
pub struct BarConfig {
    pub amount: u32,
    pub gap: f32,
    #[serde(default = "default_bar_alpha")]
    pub bar_alpha: f32,
}

fn default_bar_alpha() -> f32 { 1.0 }

#[derive(Serialize, Deserialize, Debug)]
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

// --- Configuración de imagen oculta ---
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HiddenImageConfig {
    /// Si true, usa el wallpaper actual como imagen base (ignora path)
    #[serde(default)]
    pub use_wallpaper: bool,
    /// Ruta a una imagen fija (solo si use_wallpaper = false)
    pub path: Option<String>,
    /// Efecto a aplicar sobre la imagen revelada
    #[serde(default)]
    pub effect: HiddenImageEffect,
    /// Modo de mezcla (por ahora solo "Reveal")
    #[serde(default)]
    pub blend_mode: BlendMode,
    /// Directorio que contiene las versiones "xray" de las imágenes (mismo nombre que wallpaper)
    pub xray_images_dir: Option<String>,
    /// Directorio donde se almacenan los wallpapers (opcional, para búsqueda adicional)
    pub wallpapers_dir: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum HiddenImageEffect {
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

impl Default for HiddenImageEffect {
    fn default() -> Self {
        HiddenImageEffect::None
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum BlendMode {
    Reveal,
}

impl Default for BlendMode {
    fn default() -> Self {
        BlendMode::Reveal
    }
}

// --- Funciones auxiliares existentes ---
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