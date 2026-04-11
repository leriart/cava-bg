use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub bars: BarsConfig,
    pub colors: ColorsConfig,
    pub smoothing: SmoothingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub framerate: u32,
    pub background_color: Color,
    pub autosens: Option<bool>,
    pub sensitivity: Option<u32>,
    pub preferred_output: Option<String>,
    #[serde(default = "default_auto_detect")]
    pub auto_detect_wallpaper_changes: bool,
    #[serde(default = "default_wallpaper_check_interval")]
    pub wallpaper_check_interval: u32,
    #[serde(default = "default_auto_colors")]
    pub auto_colors: bool,
}

fn default_auto_detect() -> bool { true }
fn default_wallpaper_check_interval() -> u32 { 5 }
fn default_auto_colors() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarsConfig {
    pub amount: u32,
    pub gap: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorsConfig {
    #[serde(flatten)]
    pub colors: HashMap<String, Color>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmoothingConfig {
    pub monstercat: Option<u32>,
    pub waves: Option<u32>,
    pub noise_reduction: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    Hex(String),
    HexWithAlpha { hex: String, alpha: f32 },
}

impl Color {
    pub fn to_array(&self) -> [f32; 4] {
        match self {
            Color::Hex(hex) => {
                let (r, g, b) = parse_hex_color(hex);
                [r, g, b, 1.0]
            }
            Color::HexWithAlpha { hex, alpha } => {
                let (r, g, b) = parse_hex_color(hex);
                [r, g, b, *alpha]
            }
        }
    }
}

fn parse_hex_color(hex: &str) -> (f32, f32, f32) {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
        (r, g, b)
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).unwrap_or(0) as f32 / 255.0;
        (r, g, b)
    } else {
        (0.0, 0.0, 0.0)
    }
}

impl Config {
    pub fn load(config_path: &Option<PathBuf>) -> Result<Self> {
        if let Some(path) = config_path {
            if path.exists() {
                return Self::load_from_path(path);
            } else {
                return Err(anyhow::anyhow!("Config file not found: {:?}", path));
            }
        }
        let config_paths = vec![
            PathBuf::from("config.toml"),
            dirs::config_dir()
                .map(|mut p| {
                    p.push("cava-bg");
                    p.push("config.toml");
                    p
                })
                .unwrap_or_else(|| PathBuf::from("config.toml")),
        ];
        for path in config_paths {
            if path.exists() {
                return Self::load_from_path(&path);
            }
        }
        Ok(Self::default())
    }

    pub fn load_from_path(path: &PathBuf) -> Result<Self> {
        let config_str = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Config = toml::from_str(&config_str)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    pub fn default() -> Self {
        let mut colors = HashMap::new();
        let default_hex = vec![
            "#94e2d5", "#89dceb", "#74c7ec", "#89b4fa",
            "#cba6f7", "#f5c2e7", "#eba0ac", "#f38ba8"
        ];
        for (i, hex) in default_hex.iter().enumerate() {
            colors.insert(format!("gradient_color_{}", i+1), Color::Hex(hex.to_string()));
        }
        Config {
            general: GeneralConfig {
                framerate: 30,  // Reducido de 60 a 30 para movimiento más lento
                background_color: Color::HexWithAlpha {
                    hex: "#000000".to_string(),
                    alpha: 0.0,
                },
                autosens: Some(true),
                sensitivity: Some(100),
                preferred_output: None,
                auto_detect_wallpaper_changes: true,
                wallpaper_check_interval: 5,
                auto_colors: true,
            },
            bars: BarsConfig {
                amount: 76,
                gap: 0.1,
            },
            colors: ColorsConfig { colors },
            smoothing: SmoothingConfig {
                monstercat: Some(2),      // Máxima suavidad (2)
                waves: Some(0),
                noise_reduction: Some(0.98), // Muy alta reducción de ruido
            },
        }
    }

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
        config.push_str("raw_target = /dev/stdout\n");
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