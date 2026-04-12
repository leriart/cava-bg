mod app_config;
mod cli_help;
mod wayland_renderer;
mod wallpaper;

use anyhow::{Context, Result};
use log::info;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use app_config::Config;
use cli_help::print_help;
use wayland_renderer::WaylandRenderer;

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let config_path = if args.len() == 3 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else if args.len() == 1 {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default = PathBuf::from(format!("{}/.config/wallpaper-cava/config.toml", home));
        if default.exists() {
            default
        } else {
            PathBuf::from("config.toml")
        }
    } else {
        print_help();
        exit(0);
    };

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Unable to read config file: {:?}", config_path))?;
    let config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Error parsing config: {:?}", config_path))?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting wallpaper-cava with wgpu backend");
    let renderer = WaylandRenderer::new(config, running);
    renderer.run()?;

    Ok(())
}