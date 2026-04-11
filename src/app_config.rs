use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub general: GeneralConfig,
    pub bars: BarConfig,
    pub colors: HashMap<String, ConfigColor>,
    pub smoothing: SmoothingConfig,
}

impl Default for Config {
    fn default() -> Self {
        let mut colors = HashMap::new();
        let default_colors = vec![
            "#94e2d5", "#89dceb", "#74c7ec", "#89b4fa",
            "#cba6f7", "#f5c2e7", "#eba0ac", "#f38ba8"
        ];
        for (i, hex) in default_colors.iter().enumerate() {
            colors.insert(
                format!("gradient_color_{}", i + 1),
                ConfigColor::Simple(hex.to_string()),
            );
        }
        Config {
            general: GeneralConfig::default(),
            bars: BarConfig::default(),
            colors,
            smoothing: SmoothingConfig::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GeneralConfig {
    pub framerate: u32,
    pub background_color: ConfigColor,
    pub autosens: Option<bool>,
    pub sensitivity: Option<f32>,
    pub preferred_output: Option<String>,
    #[serde(default = "default_auto_colors")]
    pub auto_colors: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            framerate: 60,
            background_color: ConfigColor::Simple("#000000".to_string()),
            autosens: Some(true),
            sensitivity: Some(1.0),
            preferred_output: None,
            auto_colors: true,
        }
    }
}

fn default_auto_colors() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BarConfig {
    pub amount: u32,
    pub gap: f32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            amount: 8,
            gap: 0.1,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SmoothingConfig {
    pub monstercat: Option<f32>,
    pub waves: Option<i32>,
    pub noise_reduction: Option<f32>,
}

impl Default for SmoothingConfig {
    fn default() -> Self {
        Self {
            monstercat: Some(1.0),
            waves: Some(0),
            noise_reduction: Some(0.8),
        }
    }
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CavaConfig {
    pub general: CavaGeneralConfig,
    pub smoothing: CavaSmoothingConfig,
    pub output: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CavaGeneralConfig {
    pub framerate: u32,
    pub bars: u32,
    pub autosens: Option<bool>,
    pub sensitivity: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CavaSmoothingConfig {
    pub monstercat: Option<f32>,
    pub waves: Option<i32>,
    pub noise_reduction: Option<f32>,
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

impl Config {
    pub fn to_cava_raw_config(&self) -> String {
        let mut config = String::new();
        config.push_str("[general]\n");
        config.push_str(&format!("framerate = {}\n", self.general.framerate));
        if let Some(autosens) = self.general.autosens {
            config.push_str(&format!("autosens = {}\n", if autosens { 1 } else { 0 }));
        }
        if let Some(sensitivity) = self.general.sensitivity {
            config.push_str(&format!("sensitivity = {}\n", sensitivity));
        }
        config.push_str("\n[output]\n");
        config.push_str(&format!("bars = {}\n", self.bars.amount));
        config.push_str("method = raw\n");
        // raw_target eliminado -> cava escribe a stdout por defecto
        config.push_str("bit_format = 16bit\n");
        config.push_str("\n[smoothing]\n");
        if let Some(monstercat) = self.smoothing.monstercat {
            config.push_str(&format!("monstercat = {}\n", monstercat));
        }
        if let Some(waves) = self.smoothing.waves {
            config.push_str(&format!("waves = {}\n", waves));
        }
        if let Some(noise_reduction) = self.smoothing.noise_reduction {
            config.push_str(&format!("noise_reduction = {:.2}\n", noise_reduction));
        }
        config
    }
}