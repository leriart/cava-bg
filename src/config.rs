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
    pub fn load() -> Result<Self> {
        let config_paths = vec![
            PathBuf::from("config.toml"),
            dirs::config_dir()
                .map(|mut p| {
                    p.push("cavabg");
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
        Config {
            general: GeneralConfig {
                framerate: 60,
                background_color: Color::HexWithAlpha { 
                    hex: "#000000".to_string(), 
                    alpha: 0.0 
                },
                autosens: Some(true),
                sensitivity: Some(100),
                preferred_output: None,
            },
            bars: BarsConfig {
                amount: 76,
                gap: 0.1,
            },
            colors: {
                let mut colors = HashMap::new();
                colors.insert("gradient_color_1".to_string(), Color::Hex("#94e2d5".to_string()));
                colors.insert("gradient_color_2".to_string(), Color::Hex("#89dceb".to_string()));
                colors.insert("gradient_color_3".to_string(), Color::Hex("#74c7ec".to_string()));
                colors.insert("gradient_color_4".to_string(), Color::Hex("#89b4fa".to_string()));
                colors.insert("gradient_color_5".to_string(), Color::Hex("#cba6f7".to_string()));
                colors.insert("gradient_color_6".to_string(), Color::Hex("#f5c2e7".to_string()));
                colors.insert("gradient_color_7".to_string(), Color::Hex("#eba0ac".to_string()));
                colors.insert("gradient_color_8".to_string(), Color::Hex("#f38ba8".to_string()));
                ColorsConfig { colors }
            },
            smoothing: SmoothingConfig {
                monstercat: Some(0),
                waves: Some(0),
                noise_reduction: Some(0.77),
            },
        }
    }
}