mod cli;
mod config;
mod cava_manager;
mod wallpaper;
mod renderer;
mod wayland_renderer;

use anyhow::{Context, Result};
use log::{error, info};
use std::process::Command;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    if cli.version {
        cli::Cli::show_version();
        return Ok(());
    }

    cli.init_logging();

    // Cargar configuración
    let mut config = config::Config::load(&cli.config).context("Failed to load config")?;

    if cli.test_config {
        println!("Testing configuration and wallpaper analysis...");
        println!("Config loaded: framerate={}, bars={}", config.general.framerate, config.bars.amount);
        if config.general.auto_colors {
            match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                Ok(colors) => {
                    println!("Generated {} colors from wallpaper:", colors.len());
                    for (i, c) in colors.iter().enumerate() {
                        let hex = format!("#{:02x}{:02x}{:02x}", (c[0]*255.0) as u8, (c[1]*255.0) as u8, (c[2]*255.0) as u8);
                        println!("  {}: {} {:?}", i+1, hex, c);
                    }
                }
                Err(e) => println!("Failed to generate colors: {}", e),
            }
        }
        return Ok(());
    }

    // Verificar que cava está instalado
    if Command::new("cava").arg("--version").output().is_err() {
        eprintln!("cava is not installed. Please install it first.");
        eprintln!("  Arch: sudo pacman -S cava");
        eprintln!("  Debian/Ubuntu: sudo apt install cava");
        eprintln!("  Fedora: sudo dnf install cava");
        return Ok(());
    }

    info!("Starting cava-bg v{}", env!("CARGO_PKG_VERSION"));

    // Generar colores automáticos si está activado
    if config.general.auto_colors {
        info!("Auto-colors enabled, analyzing wallpaper...");
        match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
            Ok(gradient) => {
                // Reemplazar colores en la configuración
                config.colors.colors.clear();
                for (i, &[r,g,b,a]) in gradient.iter().enumerate() {
                    let hex = format!("#{:02x}{:02x}{:02x}", (r*255.0) as u8, (g*255.0) as u8, (b*255.0) as u8);
                    config.colors.colors.insert(
                        format!("gradient_color_{}", i+1),
                        config::Color::HexWithAlpha { hex, alpha: a },
                    );
                }
                info!("Generated {} gradient colors from wallpaper", gradient.len());
            }
            Err(e) => {
                error!("Failed to extract colors from wallpaper: {}", e);
                info!("Using default colors");
            }
        }
    }

    // Inicializar cava manager
    let mut cava_manager = cava_manager::CavaManager::new(&config)
        .context("Failed to start cava manager")?;
    let cava_reader = cava_manager.take_reader()
        .context("Failed to get cava reader")?;

    // Configurar manejador de Ctrl+C: termina inmediatamente (como wallpaper-cava)
    ctrlc::set_handler(|| {
        std::process::exit(0);
    }).expect("Failed to set Ctrl+C handler");

    // Intentar iniciar el renderer Wayland (funcional)
    let wayland_renderer = wayland_renderer::WaylandRenderer::new(config.clone(), cava_reader);
    if let Err(e) = wayland_renderer.run() {
        error!("Wayland renderer failed: {}", e);
        info!("Falling back to terminal mode...");
        let mut terminal_renderer = renderer::Renderer::new(config, cava_manager)?;
        terminal_renderer.run()?;
    }

    Ok(())
}