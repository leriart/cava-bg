use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// use crate::wallpaper::WallpaperAnalyzer;

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

fn default_auto_detect() -> bool {
    true
}

fn default_wallpaper_check_interval() -> u32 {
    5
}

fn default_auto_colors() -> bool {
    true
}

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

// CAVA configuration structures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CavaConfig {
    pub general: CavaGeneralConfig,
    pub smoothing: CavaSmoothingConfig,
    pub output: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CavaGeneralConfig {
    pub framerate: u32,
    pub bars: u32,
    pub autosens: Option<bool>,
    pub sensitivity: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CavaSmoothingConfig {
    pub monstercat: Option<u32>,
    pub waves: Option<u32>,
    pub noise_reduction: Option<f32>,
}

impl Config {
    pub fn load(config_path: &Option<std::path::PathBuf>) -> Result<Self> {
        // If config path is provided, use it
        if let Some(path) = config_path {
            if path.exists() {
                return Self::load_from_path(path);
            } else {
                return Err(anyhow::anyhow!("Config file not found: {:?}", path));
            }
        }
        
        // Otherwise, search in default locations
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

        // Return default config if no file found
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
        // Use Catppuccin Mocha gradient as default (fallback)
        let gradient_colors = vec![
            [0.580, 0.886, 0.835, 1.0], // #94e2d5
            [0.537, 0.863, 0.922, 1.0], // #89dceb
            [0.455, 0.780, 0.925, 1.0], // #74c7ec
            [0.537, 0.706, 0.980, 1.0], // #89b4fa
            [0.796, 0.651, 0.969, 1.0], // #cba6f7
            [0.961, 0.761, 0.906, 1.0], // #f5c2e7
            [0.922, 0.627, 0.675, 1.0], // #eba0ac
            [0.953, 0.545, 0.659, 1.0], // #f38ba8
        ];

        let mut colors = HashMap::new();
        for (i, color) in gradient_colors.iter().enumerate() {
            let hex = format!(
                "#{:02x}{:02x}{:02x}",
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8
            );
            colors.insert(format!("gradient_color_{}", i + 1), Color::Hex(hex));
        }

        Config {
            general: GeneralConfig {
                framerate: 60,
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
                monstercat: Some(0),
                waves: Some(0),
                noise_reduction: Some(0.77),
            },
        }
    }

    pub fn to_cava_config(&self) -> String {
        let mut config = String::new();
        
        config.push_str(&format!("[general]\n"));
        config.push_str(&format!("framerate = {}\n", self.general.framerate));
        if let Some(autosens) = self.general.autosens {
            config.push_str(&format!("autosens = {}\n", if autosens { "1" } else { "0" }));
        }
        if let Some(sensitivity) = self.general.sensitivity {
            config.push_str(&format!("sensitivity = {}\n", sensitivity));
        }
        
        config.push_str(&format!("\n[output]\n"));
        config.push_str(&format!("bars = {}\n", self.bars.amount));
        
        config.push_str(&format!("\n[smoothing]\n"));
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
