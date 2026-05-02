use anyhow::{Context, Result};
use eframe::egui::{self, Color32, Key, RichText};
use rfd::FileDialog;
use std::env;
use std::fs;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::app_config::{
    AnimationType, BarShape, BlendMode, ColorExtractionMode, Config, LayerAnimationConfig,
    LayerChoice, LayerSourceType, MaskComputeMode, ParallaxLayerConfig, ParallaxMode,
    ParallaxProfile, ProfileSource, VisualizationMode,
};
use crate::wallpaper::WallpaperAnalyzer;

pub fn run_config_gui(config_path: &Path) -> Result<()> {
    if !config_path.exists() {
        crate::create_default_config(config_path).context("Failed to create default config")?;
    }

    let app = match ConfigEditorApp::load(config_path) {
        Ok(app) => app,
        Err(e) => {
            eprintln!(
                "Warning: Could not parse config ({}). Creating a fresh config...",
                e
            );
            let backup = config_path.with_extension("toml.legacy");
            if let Err(copy_err) = fs::copy(config_path, &backup) {
                eprintln!(
                    "Warning: Could not back up old config to {:?}: {}",
                    backup, copy_err
                );
            } else {
                eprintln!("Backed up old config to {:?}", backup);
            }
            crate::create_default_config(config_path)
                .context("Failed to replace with default config")?;
            ConfigEditorApp::load(config_path)?
        }
    };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cava-BG Configuration")
            .with_inner_size([1220.0, 860.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Cava-BG Configuration",
        native_options,
        Box::new(|cc: &eframe::CreationContext| {
            // Load system fallback fonts for Unicode symbol coverage.
            // NotoSansSymbols2 covers U+2000-U+2BFF (misc symbols, arrows, geometric shapes).
            // Falls back to SymbolsNerdFont if NotoSansSymbols2 isn't available.
            let mut fonts = egui::FontDefinitions::default();

            let symbol_fonts = [
                "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
                "/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf",
            ];

            for path in &symbol_fonts {
                if let Ok(data) = std::fs::read(path) {
                    let name = format!("symbols-{}", path.rsplit('/').next().unwrap_or("fallback"));
                    fonts
                        .font_data
                        .insert(name.clone(), egui::FontData::from_owned(data));
                    // Prepend to Proportional group as fallback (lower priority than main font)
                    if let Some(families) = fonts.families.get_mut(&egui::FontFamily::Proportional)
                    {
                        families.insert(0, name);
                    }
                    log::info!("Loaded symbol font: {}", path);
                    break;
                }
            }

            cc.egui_ctx.set_fonts(fonts);
            Box::new(app)
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to open GUI: {}", e))?;

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigTab {
    Audio,
    Visualizer,
    Effects,
    Colors,
    Xray,
    Parallax,
    Performance,
    Advanced,
}

#[derive(Default, Clone)]
struct DaemonStatus {
    running: bool,
    pid: Option<i32>,
    pid_file: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ColorUpdate {
    colors: Vec<[f32; 4]>,
    bar_alpha: Option<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RuntimeOutputInfo {
    name: String,
}

fn runtime_color_socket_path(config_path: &Path) -> PathBuf {
    let dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("runtime-color-update.sock")
}

fn runtime_outputs_path(config_path: &Path) -> PathBuf {
    let dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("runtime-outputs.json")
}

fn load_runtime_output_names(config_path: &Path) -> Vec<String> {
    let path = runtime_outputs_path(config_path);
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let Ok(outputs) = serde_json::from_str::<Vec<RuntimeOutputInfo>>(&content) else {
        return Vec::new();
    };

    outputs.into_iter().map(|o| o.name).collect()
}

struct ConfigEditorApp {
    config_path: PathBuf,
    config: Config,
    saved_snapshot: String,
    tab: ConfigTab,
    status: String,
    show_apply_confirm: bool,
    show_reset_confirm: bool,
    wallpapers_dir_input: String,
    xray_dir_input: String,
    layers_dir_input: String,
    base_layer_input: String,
    reveal_layer_input: String,
    daemon_status: DaemonStatus,
    detected_outputs: Vec<String>,
    selected_output_override: String,
    copy_output_target: String,
    last_live_push_at: Option<Instant>,
    live_push_snapshot: String,
    preset_name_input: String,
    available_presets: Vec<String>,
    pending_file_result: Option<Arc<Mutex<Option<String>>>>,
    /// When true, next pending file result is for xray hidden_image path instead of parallax layers.
    pending_file_for_xray: bool,
    /// Parallax profile management
    profiles_dir_input: String,
    available_profiles: Vec<String>,
    selected_profile: String,
    create_profile_dialog: bool,
    create_profile_name: String,
    create_profile_use_wallpaper: bool,
}

impl ConfigEditorApp {
    fn load(config_path: &Path) -> Result<Self> {
        let config_str = fs::read_to_string(config_path)
            .with_context(|| format!("Could not read {}", config_path.display()))?;
        let mut config: Config = toml::from_str(&config_str)
            .with_context(|| format!("Could not parse {}", config_path.display()))?;
        config.normalize_compat_fields();

        let saved_snapshot = toml::to_string_pretty(&config)?;
        let wallpapers_dir_input = config
            .wallpaper
            .wallpapers_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let xray_dir_input = config.xray.images_dir.clone().unwrap_or_default();
        let layers_dir_input = config
            .parallax
            .profiles_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let base_layer_input = config
            .layers
            .as_ref()
            .map(|l| l.base.source.path.clone())
            .or_else(|| {
                if !config.xray.base_layer_path.is_empty() {
                    Some(config.xray.base_layer_path.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let reveal_layer_input = config
            .layers
            .as_ref()
            .map(|l| l.reveal.source.path.clone())
            .or_else(|| {
                if !config.xray.reveal_layer_path.is_empty() {
                    Some(config.xray.reveal_layer_path.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let profiles_dir_input = config
            .parallax
            .profiles_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let default_selected_profile = config.parallax.active_profile.clone().unwrap_or_default();

        Ok(Self {
            config_path: config_path.to_path_buf(),
            config,
            saved_snapshot: saved_snapshot.clone(),
            tab: ConfigTab::Audio,
            status: "Ready".to_string(),
            show_apply_confirm: false,
            show_reset_confirm: false,
            wallpapers_dir_input,
            xray_dir_input,
            layers_dir_input,
            base_layer_input,
            reveal_layer_input,
            daemon_status: Self::daemon_status(),
            detected_outputs: load_runtime_output_names(config_path),
            selected_output_override: String::new(),
            copy_output_target: String::new(),
            last_live_push_at: None,
            live_push_snapshot: saved_snapshot.clone(),
            preset_name_input: String::new(),
            available_presets: list_presets(),
            profiles_dir_input,
            available_profiles: Vec::new(),
            selected_profile: default_selected_profile,
            pending_file_result: None,
            pending_file_for_xray: false,
            create_profile_dialog: false,
            create_profile_name: String::new(),
            create_profile_use_wallpaper: true,
        })
    }

    fn check_pending_file(&mut self) -> Option<String> {
        if let Some(ref result) = self.pending_file_result {
            if let Ok(mut guard) = result.lock() {
                if let Some(path) = guard.take() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn apply_visuals(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = egui::Rounding::same(10.0);
        visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.active.rounding = egui::Rounding::same(8.0);
        visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
        visuals.selection.bg_fill = Color32::from_rgb(108, 92, 231);
        visuals.widgets.active.bg_fill = Color32::from_rgb(65, 105, 225);
        ctx.set_visuals(visuals);
    }

    fn sync_inputs_into_config(&mut self) {
        self.config.wallpaper.wallpapers_dir = if self.wallpapers_dir_input.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(self.wallpapers_dir_input.trim()))
        };

        self.config.parallax.profiles_dir = if self.profiles_dir_input.trim().is_empty() {
            Some(
                self.config_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join("parallax"),
            )
        } else {
            Some(PathBuf::from(self.profiles_dir_input.trim()))
        };
        self.config.parallax.active_profile = if self.selected_profile.trim().is_empty() {
            None
        } else {
            Some(self.selected_profile.trim().to_string())
        };

        // layers_dir is for scan only, not persisted to parallax config

        self.config.xray.images_dir = if self.xray_dir_input.trim().is_empty() {
            None
        } else {
            Some(self.xray_dir_input.trim().to_string())
        };

        let has_base = !self.base_layer_input.trim().is_empty();
        let has_reveal = !self.reveal_layer_input.trim().is_empty();
        if has_base || has_reveal {
            ensure_layers_exist(&mut self.config);
            if let Some(layers) = self.config.layers.as_mut() {
                if has_base {
                    layers.base.source.path = self.base_layer_input.trim().to_string();
                    layers.base.source.r#type = infer_source_type(&layers.base.source.path);
                }
                if has_reveal {
                    layers.reveal.source.path = self.reveal_layer_input.trim().to_string();
                    layers.reveal.source.r#type = infer_source_type(&layers.reveal.source.path);
                }
                layers.base.max_buffered_frames = self.config.advanced.layer_cache_size.max(1);
                layers.reveal.max_buffered_frames = self.config.advanced.layer_cache_size.max(1);
            }
        } else {
            // If both inputs cleared but layers exist, don't delete them — keep user data safe
        }

        self.config.wallpaper.sync_interval_seconds =
            self.config.wallpaper.sync_interval_seconds.max(0.001);
        self.config.general.framerate = self.config.general.framerate.max(1);
        self.config.advanced.frame_rate_limit = self.config.advanced.frame_rate_limit.max(1);
        self.config.advanced.layer_cache_size = self.config.advanced.layer_cache_size.max(1);
        self.config.audio.bar_count = self.config.audio.bar_count.max(1);
        self.config.audio.smoothing = self.config.audio.smoothing.clamp(0.0, 1.0);
        self.config.audio.max_bar_height = self.config.audio.max_bar_height.max(1.0);
        self.config.audio.min_bar_height = self.config.audio.min_bar_height.max(0.0);
        self.config.display.opacity = self.config.display.opacity.clamp(0.0, 1.0);

        if self.config.colors.palette.is_empty() {
            self.config.colors.palette.push([1.0, 0.0, 1.0, 1.0]);
        }

        // Sync xray path fields from input buffers into config
        if !self.base_layer_input.trim().is_empty() {
            self.config.xray.base_layer_path = self.base_layer_input.trim().to_string();
        }
        if !self.reveal_layer_input.trim().is_empty() {
            self.config.xray.reveal_layer_path = self.reveal_layer_input.trim().to_string();
        }
    }

    fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.config.wallpaper.sync_interval_seconds == 0.0 {
            errors.push("Sync interval must be at least 1ms.".to_string());
        }
        if self.config.general.framerate == 0 {
            errors.push("Frame rate must be greater than 0.".to_string());
        }
        if self.config.audio.bar_count == 0 {
            errors.push("Bar count must be at least 1.".to_string());
        }
        if self.config.audio.max_bar_height < self.config.audio.min_bar_height {
            errors.push("Max bar height must be greater or equal to min bar height.".to_string());
        }

        errors
    }

    fn static_colors_from_config(config: &Config) -> Vec<[f32; 4]> {
        if !config.colors.palette.is_empty() {
            return config.colors.palette.clone();
        }
        vec![crate::app_config::array_from_config_color(
            config.audio.bar_color.clone(),
        )]
    }

    fn push_live_color_update(&self) {
        let socket_path = runtime_color_socket_path(&self.config_path);
        if !socket_path.exists() {
            return;
        }

        // Siempre mandamos la palette si no está vacía — los gradient_colors
        // son para el efecto de gradiente en las barras, no deben bloquear
        // los colores manuales o extraídos.
        let colors = if !self.config.colors.palette.is_empty() {
            self.config.colors.palette.clone()
        } else if self.config.colors.extract_from_wallpaper {
            self.config.colors.palette.clone()
        } else {
            Self::static_colors_from_config(&self.config)
        };

        let update = ColorUpdate {
            colors,
            bar_alpha: Some(self.config.audio.bar_alpha),
        };

        let payload = match serde_json::to_vec(&update) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Could not serialize ColorUpdate payload: {e}");
                return;
            }
        };

        let socket = match UnixDatagram::unbound() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Could not create unix datagram socket: {e}");
                return;
            }
        };

        if let Err(e) = socket.send_to(&payload, &socket_path) {
            eprintln!(
                "Could not push direct color update to {}: {}",
                socket_path.display(),
                e
            );
        }
    }

    fn persist(&mut self) -> Result<()> {
        self.sync_inputs_into_config();

        // Si no estamos en modo wallpaper extraction, desactivamos dynamic_colors
        // para que al reiniciar el daemon no pierda los colores elegidos.
        if !self.config.colors.extract_from_wallpaper {
            self.config.general.dynamic_colors = false;
        }

        let content = toml::to_string_pretty(&self.config).context("Could not serialize config")?;
        fs::write(&self.config_path, &content)
            .with_context(|| format!("Could not save {}", self.config_path.display()))?;
        self.saved_snapshot = content.clone();
        self.live_push_snapshot = content;
        self.push_live_color_update();
        Ok(())
    }

    fn is_dirty(&self) -> bool {
        toml::to_string_pretty(&self.config)
            .map(|s| s != self.saved_snapshot)
            .unwrap_or(true)
    }

    fn reset_defaults(&mut self) {
        self.config = Config::default();
        self.wallpapers_dir_input.clear();
        self.xray_dir_input.clear();
        self.layers_dir_input.clear();
        self.base_layer_input.clear();
        self.reveal_layer_input.clear();
        self.status = "Defaults restored. Save or Apply to persist changes.".to_string();
    }

    fn save_preset(&mut self, name: &str) -> Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Preset name cannot be empty");
        }
        self.sync_inputs_into_config();
        let content = toml::to_string_pretty(&self.config).context("Could not serialize preset")?;
        let path = preset_dir().join(format!("{trimmed}.toml"));
        fs::write(&path, content).with_context(|| format!("Could not write {}", path.display()))?;
        self.available_presets = list_presets();
        Ok(())
    }

    fn load_preset(&mut self, name: &str) -> Result<()> {
        let path = preset_dir().join(format!("{}.toml", name.trim()));
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Could not read preset {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&content)
            .with_context(|| format!("Could not parse preset {}", path.display()))?;
        cfg.normalize_compat_fields();
        self.config = cfg;
        self.wallpapers_dir_input = self
            .config
            .wallpaper
            .wallpapers_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        self.xray_dir_input = self.config.xray.images_dir.clone().unwrap_or_default();
        self.layers_dir_input = self
            .config
            .parallax
            .profiles_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        self.base_layer_input = self
            .config
            .layers
            .as_ref()
            .map(|l| l.base.source.path.clone())
            .unwrap_or_default();
        self.reveal_layer_input = self
            .config
            .layers
            .as_ref()
            .map(|l| l.reveal.source.path.clone())
            .unwrap_or_default();
        Ok(())
    }

    fn daemon_pid_path() -> PathBuf {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{home}/.config/cava-bg/daemon.pid"))
    }

    fn daemon_status() -> DaemonStatus {
        let pid_file = Self::daemon_pid_path();
        let pid = read_pid_from_file(&pid_file);
        let running = pid.is_some_and(process_exists);
        DaemonStatus {
            running,
            pid,
            pid_file,
        }
    }

    fn refresh_daemon_status(&mut self) {
        self.daemon_status = Self::daemon_status();
    }

    fn execute_daemon_command(&mut self, command: &str) -> Result<String> {
        let exe = env::current_exe().context("Could not resolve executable path")?;
        let output = Command::new(exe)
            .arg(command)
            .arg("--config")
            .arg(&self.config_path)
            .output()
            .with_context(|| format!("Failed to execute 'cava-bg {command}'"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.status.success() {
            Ok(if stdout.is_empty() {
                format!("Daemon command '{command}' executed successfully.")
            } else {
                stdout
            })
        } else if stderr.is_empty() {
            anyhow::bail!("Daemon command '{command}' failed.")
        } else {
            anyhow::bail!(stderr)
        }
    }

    fn restart_daemon(&mut self) -> Result<()> {
        let _ = self.execute_daemon_command("off");
        let on_msg = self.execute_daemon_command("on")?;
        self.refresh_daemon_status();
        // Después de reiniciar, esperamos un poco y mandamos los colores
        // via socket para que el nuevo daemon los reciba en caliente.
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.push_live_color_update();
        self.status = format!("Applied + restarted daemon. {on_msg}");
        Ok(())
    }

    fn maybe_push_live_update(&mut self) {
        self.refresh_daemon_status();
        if !self.daemon_status.running {
            return;
        }

        let current_snapshot = match toml::to_string_pretty(&self.config) {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("Hot-reload snapshot error: {e}");
                return;
            }
        };

        let hot_reload_tab = matches!(
            self.tab,
            ConfigTab::Audio
                | ConfigTab::Visualizer
                | ConfigTab::Effects
                | ConfigTab::Colors
                | ConfigTab::Xray
                | ConfigTab::Parallax
        );
        if !hot_reload_tab || current_snapshot == self.live_push_snapshot {
            return;
        }

        if let Some(last) = self.last_live_push_at {
            if last.elapsed() < Duration::from_millis(180) {
                return;
            }
        }

        match self.persist() {
            Ok(_) => {
                self.live_push_snapshot = current_snapshot;
                self.last_live_push_at = Some(Instant::now());
                self.status = "Auto-saved to config file (hot-reload).".to_string();
            }
            Err(e) => {
                self.status = format!("Auto-save error: {e}");
            }
        }
    }

    fn handle_save_shortcut(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(Key::S)) {
            match self.persist() {
                Ok(_) => self.status = "Configuration saved with Ctrl+S.".to_string(),
                Err(e) => self.status = format!("Save error: {e}"),
            }
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let dirty_mark = if self.is_dirty() { " *" } else { "" };
        ui.horizontal(|ui| {
            ui.heading(format!("Cava-BG Configuration{dirty_mark}"));
            ui.label(RichText::new("Expanded editor").italics().small());
        });
        ui.separator();
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            tab_button(ui, &mut self.tab, ConfigTab::Audio, "\u{23EF} Audio Source");
            tab_button(
                ui,
                &mut self.tab,
                ConfigTab::Visualizer,
                "\u{2590}\u{258C} Visualizer",
            );
            tab_button(ui, &mut self.tab, ConfigTab::Effects, "\u{2726} Effects");
            tab_button(ui, &mut self.tab, ConfigTab::Colors, "\u{1F3A8} Colors");
            tab_button(ui, &mut self.tab, ConfigTab::Parallax, "\u{1F300} Parallax");
            tab_button(ui, &mut self.tab, ConfigTab::Xray, "\u{229E} X-Ray");
            tab_button(
                ui,
                &mut self.tab,
                ConfigTab::Performance,
                "\u{26A1} Performance",
            );
            tab_button(ui, &mut self.tab, ConfigTab::Advanced, "\u{2699} Advanced");
        });
        ui.separator();
    }

    fn section_visualizer(&mut self, ui: &mut egui::Ui) {
        ui.heading("\u{2590}\u{258C} Visualizer Mode & Layout");
        ui.separator();

        // ── Mode & Shape ──
        ui.collapsing("\u{25B6} Mode & Shape", |ui| {
            ui.horizontal(|ui| {
                ui.label("Visualization mode:")
                    .on_hover_text("How the audio spectrum is visually represented.");
                visualization_mode_combo(
                    ui,
                    &mut self.config.audio.visualization_mode,
                    "visualization_mode_combo",
                );
            });
            ui.label(
                RichText::new(self.describe_visualization_mode())
                    .small()
                    .italics(),
            );

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Bar shape:")
                    .on_hover_text("The geometric shape of each individual bar/band element.");
                bar_shape_combo(ui, &mut self.config.audio.bar_shape, "bar_shape_combo");
            });
            ui.label(RichText::new(self.describe_bar_shape()).small().italics());
        });

        // ── Dimensions & Spacing ──
        ui.collapsing("\u{25B6} Dimensions & Spacing", |ui| {
            ui.add(egui::Slider::new(&mut self.config.audio.bar_count, 10..=256).text("Bar count"))
                .on_hover_text("Number of frequency bands. More bars = finer detail but lower performance. Start around 32-64.");
            ui.add(egui::Slider::new(&mut self.config.audio.bar_width, 1.0..=50.0).text("Bar width (px)"))
                .on_hover_text("Width of each bar in pixels.");
            ui.add(egui::Slider::new(&mut self.config.audio.bar_spacing, 0.0..=20.0).text("Bar spacing (px)"))
                .on_hover_text("Gap between adjacent bars. 0 = bars touch.");
        });

        // ── Height & Responsiveness ──
        ui.collapsing("\u{25B6} Height & Responsiveness", |ui| {
            ui.add(egui::Slider::new(&mut self.config.audio.height_scale, 0.1..=3.0).text("Height scale"))
                .on_hover_text("Overall height multiplier. >1.0 = taller (more reactive), <1.0 = shorter (subtle).");
            ui.add(egui::Slider::new(&mut self.config.audio.max_bar_height, 1.0..=1000.0).text("Max height (px)"))
                .on_hover_text("Absolute maximum bar height. Bars won't exceed this regardless of volume.");
            ui.add(egui::Slider::new(&mut self.config.audio.min_bar_height, 0.0..=200.0).text("Min height (px)"))
                .on_hover_text("Minimum bar height. Prevents bars from disappearing at low volumes.");
        });

        // ── Mode-specific contextual options ──
        match self.config.audio.visualization_mode {
            VisualizationMode::Ring => {
                ui.collapsing("\u{25B6} Ring Options", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.config.audio.radial_inner_radius, 0.0..=500.0)
                            .text("Inner radius"),
                    )
                    .on_hover_text(
                        "Radius of the ring's inner edge. Ring thickness varies with audio.",
                    );
                    ui.add(
                        egui::Slider::new(&mut self.config.audio.radial_sweep_angle, 1.0..=360.0)
                            .text("Sweep angle (\u{00B0})"),
                    )
                    .on_hover_text(
                        "Angular span of the ring. 360 = full circle, 180 = half circle.",
                    );
                });
            }
            VisualizationMode::Waveform => {
                ui.collapsing("\u{25B6} Waveform Options", |ui| {
                    ui.add(egui::Slider::new(&mut self.config.audio.waveform_line_width, 1.0..=20.0).text("Line width"))
                        .on_hover_text("Thickness of the waveform line in pixels.");
                    ui.add(egui::Slider::new(&mut self.config.audio.waveform_smoothness, 0.0..=1.0).text("Smoothness"))
                        .on_hover_text("0 = sharp polygonal waveform, 1 = maximally smoothed bezier curves between frequency points.");
                });
            }
            VisualizationMode::Blocks => {
                ui.collapsing("\u{25B6} Block Options", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.config.audio.block_size, 2.0..=50.0)
                            .text("Block size"),
                    )
                    .on_hover_text(
                        "Size of each frequency block. Small = pixel-art, large = chunky.",
                    );
                    ui.add(
                        egui::Slider::new(&mut self.config.audio.block_spacing, 0.0..=20.0)
                            .text("Block spacing"),
                    )
                    .on_hover_text("Gap between adjacent blocks.");
                });
            }
            _ => {}
        }

        // ── Corner options (only Circle/Triangle shapes) ──
        if matches!(
            self.config.audio.bar_shape,
            BarShape::Circle | BarShape::Triangle
        ) {
            ui.collapsing("\u{25B6} Corner Options", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Corner radius (px):")
                        .on_hover_text("Rounds corners of non-rectangular bar shapes.");
                    ui.add(
                        egui::DragValue::new(&mut self.config.audio.corner_radius)
                            .clamp_range(0.0f32..=64.0f32)
                            .speed(0.5),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Corner segments:").on_hover_text(
                        "Quality of curved corners. Higher = smoother but more GPU work.",
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.config.audio.corner_segments)
                            .clamp_range(2u32..=32u32)
                            .speed(1),
                    );
                });
            });
        }

        ui.separator();

        // ── Display Placement ──
        ui.collapsing("\u{25B6} Display Placement", |ui| {
            ui.horizontal(|ui| {
                ui.label("Render layer:").on_hover_text("Which compositor layer the visualizer renders on.");
                ui.selectable_value(&mut self.config.display.layer, LayerChoice::Background, "Background")
                    .on_hover_text("Behind everything (wallpaper layer)");
                ui.selectable_value(&mut self.config.display.layer, LayerChoice::Bottom, "Bottom")
                    .on_hover_text("Above background, behind windows");
            });

            ui.add(egui::Slider::new(&mut self.config.general.framerate, 1..=240).text("Frame rate (FPS)"))
                .on_hover_text("Target frames per second. Higher = smoother but more GPU/CPU usage. 30-60 is usually sufficient.");
        });

        // ── Visibility ──
        ui.collapsing("\u{25B6} Visibility", |ui| {
            ui.checkbox(&mut self.config.audio.show_visualizer, "Show visualizer")
                .on_hover_text("Toggle the visualizer on/off. Audio processing continues even when hidden.");
            if !self.config.audio.show_visualizer {
                ui.small("Visualizer is hidden. Audio processing continues in background.");
            }
            ui.checkbox(&mut self.config.parallax.visualizer_as_parallax_layer, "Use as parallax layer")
                .on_hover_text("When enabled, the visualizer becomes part of the parallax depth stack.");
            if self.config.parallax.visualizer_as_parallax_layer {
                ui.small("Visualizer will render as part of the parallax layer stack. Enable Parallax in the Parallax tab.");
            }
        });
    }

    fn section_effects(&mut self, ui: &mut egui::Ui) {
        ui.heading("\u{2726} Smoothing & Visual Effects");
        ui.separator();

        ui.collapsing("\u{25B6} Bar Smoothing", |ui| {
            ui.add(
                egui::Slider::new(&mut self.config.audio.smoothing, 0.0..=1.0)
                    .text("Bar smoothing"),
            )
            .on_hover_text(
                "Cross-frame smoothness: 0 = instant/laggy, 1 = smooth but sluggish. Try 0.7.",
            );
        });

        ui.collapsing("\u{25B6} CAVA Advanced Filters", |ui| {
            let mut mc_on = self.config.smoothing.monstercat.unwrap_or(0.0) > 0.5;
            if ui.checkbox(&mut mc_on, "Monstercat style").changed() {
                self.config.smoothing.monstercat = if mc_on { Some(1.0) } else { Some(0.0) };
            }
            ui.label("Bars fall rapidly from peaks but rise more slowly.")
                .on_hover_text("Creates the classic music-visualizer look with faster fall-off.");

            let mut wv = self.config.smoothing.waves.unwrap_or(0);
            if ui
                .add(egui::Slider::new(&mut wv, 0..=10).text("Wave harmonics"))
                .changed()
            {
                self.config.smoothing.waves = Some(wv);
            }
            ui.label("Number of CAVA wave harmonics. Higher = more complex wave shapes.")
                .on_hover_text("Causes smoother waves in the audio spectrum.");
            ui.small("Smoothing changes require restarting CAVA to take effect.");
        });
    }

    fn section_audio(&mut self, ui: &mut egui::Ui) {
        ui.heading("\u{23EF} Audio Source & Sensitivity");
        ui.separator();

        ui.collapsing("\u{25B6} Sensitivity", |ui| {
            let mut autosens_val = self.config.general.autosens.unwrap_or(true);
            if ui.checkbox(&mut autosens_val, "Automatic sensitivity").changed() {
                self.config.general.autosens = Some(autosens_val);
            }
            ui.label("Auto-adjust sensitivity based on audio levels.").on_hover_text("When enabled, sensitivity is automatically adjusted based on current audio levels. Recommended for most usage.");

            let mut sens = self.config.general.sensitivity.unwrap_or(100.0);
            ui.horizontal(|ui| {
                ui.label("Sensitivity (%)").on_hover_text("Higher values make the visualizer more reactive. 100 = default. Only used when auto-sensitivity is off.");
                if ui.add(egui::Slider::new(&mut sens, 0.0..=200.0)).changed() {
                    self.config.general.sensitivity = Some(sens);
                }
            });

            ui.checkbox(&mut self.config.general.disable_audio, "Disable audio capture")
                .on_hover_text("Completely disable audio processing. Useful for debugging or when running as a static wallpaper.");
        });
    }

    fn section_layers_parallax(&mut self, ui: &mut egui::Ui) {
        ui.heading("\u{1f300} Parallax System \u{2014} Multi-Layer Depth Effect");

        ui.checkbox(&mut self.config.parallax.enabled, "Enable Parallax")
            .on_hover_text("Enable multi-layered parallax scrolling.\nLayers move at different speeds creating a 3D depth effect.\nRequires one or more image layers configured below.");
        if !self.config.parallax.enabled {
            ui.label("\u{2139} Parallax is disabled. Enable it above to configure layers and depth settings.");
            return;
        }

        ui.separator();
        ui.label(
            egui::RichText::new("\u{1f4c1} Parallax Profiles")
                .strong()
                .size(14.0),
        );
        ui.small("Profiles bundle layer images and settings into reusable presets.");
        path_picker_row(ui, "Profiles directory", &mut self.profiles_dir_input, true);

        if ui.button("Refresh profiles")
            .on_hover_text("Scan the profiles directory for parallax profiles.\nEach profile is a subfolder containing a parallax.toml.")
            .clicked() {
            let dir = if self.profiles_dir_input.trim().is_empty() {
                self.config_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join("parallax")
            } else {
                PathBuf::from(self.profiles_dir_input.trim())
            };
            self.available_profiles = ParallaxProfile::discover_profiles(&dir);
            if self.available_profiles.is_empty() {
                self.status = format!("No profiles found in {:?}", dir);
            } else {
                self.status = format!(
                    "Found {} profile(s): {}",
                    self.available_profiles.len(),
                    self.available_profiles.join(", ")
                );
            }
        }
        ui.small("Scans the profiles directory for subfolders containing a parallax.toml.");

        if !self.available_profiles.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Active Profile:");
                egui::ComboBox::from_id_source("active_profile_combo")
                    .selected_text(if self.selected_profile.is_empty() {
                        "(none)"
                    } else {
                        &self.selected_profile
                    })
                    .show_ui(ui, |ui: &mut egui::Ui| {
                        for p in &self.available_profiles {
                            if ui
                                .selectable_value(&mut self.selected_profile, p.clone(), p.as_str())
                                .clicked()
                            {
                                self.config.parallax.layers.clear();
                                self.config.parallax.active_profile = Some(p.clone());
                                self.status = format!("Selected profile '{}'", p);
                            }
                        }
                    });
            });
        }

        ui.horizontal(|ui| {
            ui.label("Profile source:")
                .on_hover_text("• Normal — Use the profile selected from the dropdown\n• From Wallpaper — Auto-select a profile matching the current wallpaper name");
            egui::ComboBox::from_id_source("profile_source_combo")
                .selected_text(match self.config.parallax.profile_source {
                    ProfileSource::Normal => "Normal",
                    ProfileSource::FromWallpaper => "From Wallpaper",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.parallax.profile_source,
                        ProfileSource::Normal,
                        "Normal",
                    );
                    ui.selectable_value(
                        &mut self.config.parallax.profile_source,
                        ProfileSource::FromWallpaper,
                        "From Wallpaper",
                    );
                });
        });

        if ui
            .button("󰺷 Create New Profile")
            .on_hover_text("Create a new parallax profile from an image or wallpaper.")
            .clicked()
        {
            self.create_profile_dialog = true;
            self.create_profile_name.clear();
        }

        // Create profile dialog
        if self.create_profile_dialog {
            ui.separator();
            ui.heading("Create New Parallax Profile");
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.create_profile_name);
            });
            ui.checkbox(
                &mut self.create_profile_use_wallpaper,
                "Use current wallpaper image",
            );
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    let name = self.create_profile_name.trim().to_string();
                    if name.is_empty() {
                        self.status = "Profile name cannot be empty.".to_string();
                    } else {
                        let dir = if self.profiles_dir_input.trim().is_empty() {
                            self.config_path
                                .parent()
                                .unwrap_or(Path::new(""))
                                .join("parallax")
                        } else {
                            PathBuf::from(self.profiles_dir_input.trim())
                        };

                        let result: Result<ParallaxProfile, anyhow::Error> =
                            if self.create_profile_use_wallpaper {
                                // Use current wallpaper
                                let wallpaper = WallpaperAnalyzer::find_wallpaper();
                                if let Some(wp_path) = wallpaper {
                                    ParallaxProfile::create(&dir, &name, &wp_path)
                                } else {
                                    self.status = "Could not detect current wallpaper.".to_string();
                                    return;
                                }
                            } else {
                                // Pick a file
                                if let Some(path) = FileDialog::new()
                                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                                    .pick_file()
                                {
                                    ParallaxProfile::create(&dir, &name, &path)
                                } else {
                                    self.status = "No file selected.".to_string();
                                    return;
                                }
                            };

                        match result {
                            Ok(_) => {
                                self.available_profiles = ParallaxProfile::discover_profiles(&dir);
                                self.selected_profile = name;
                                self.status =
                                    format!("Created profile '{}'", &self.selected_profile);
                            }
                            Err(e) => {
                                self.status = format!("Failed to create profile: {e}");
                            }
                        }
                        self.create_profile_dialog = false;
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.create_profile_dialog = false;
                }
            });
        }

        if !self.selected_profile.is_empty() && ui.button("󰚠 Delete selected profile").clicked()
        {
            let dir = if self.profiles_dir_input.trim().is_empty() {
                self.config_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join("parallax")
            } else {
                PathBuf::from(self.profiles_dir_input.trim())
            };
            let profile_dir = dir.join(&self.selected_profile);
            if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
                self.status = format!("Failed to delete profile: {e}");
            } else {
                self.available_profiles = ParallaxProfile::discover_profiles(&dir);
                if self.selected_profile
                    == self.config.parallax.active_profile.as_deref().unwrap_or("")
                {
                    self.config.parallax.active_profile = None;
                }
                self.selected_profile.clear();
                self.status = "Deleted profile directory".to_string();
            }
        }

        ui.separator();
        ui.heading("Per-Layer Settings");

        // Load profile layers into the layer list if profile is active but layers are empty
        let has_profile = !self.selected_profile.is_empty();
        if has_profile && self.config.parallax.layers.is_empty() {
            let dir = if self.profiles_dir_input.trim().is_empty() {
                self.config_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join("parallax")
            } else {
                PathBuf::from(self.profiles_dir_input.trim())
            };
            if let Ok(profile) = ParallaxProfile::load(&dir, &self.selected_profile) {
                let _base_dir = dir.join(&self.selected_profile);
                for layer_file in &profile.layers {
                    let source = profile.resolve_layer(&dir, layer_file);
                    let mut layer_cfg = profile.layer_config(layer_file);
                    if layer_cfg.source.is_empty() {
                        layer_cfg.source = source.to_string_lossy().to_string();
                    }
                    self.config.parallax.layers.push(layer_cfg);
                }
            }
        }

        // Show common settings regardless of profile mode
        ui.label(
            egui::RichText::new("\u{2699} General Settings")
                .strong()
                .size(14.0),
        );
        ui.horizontal(|ui| {
            ui.label("Parallax Mode:")
                .on_hover_text("How parallax layers respond to input:\n\nStatic — No movement\nMouse — Track cursor position\nAudio — React to audio amplitude and bands\nAudio + Mouse — React to both");
            parallax_mode_combo(ui, &mut self.config.parallax.mode, "parallax_mode_combo");
        });
        ui.checkbox(&mut self.config.parallax.enable_3d_depth, "Enable 3D Depth")
            .on_hover_text("Apply perspective depth scaling to layers.\nLayers farther back appear smaller and move slower.");

        ui.separator();
        ui.label(
            egui::RichText::new("\u{1f5d0} Render Layer")
                .strong()
                .size(14.0),
        );
        ui.small("Which compositor layer the renderer attaches to:");

        ui.separator();
        ui.label(
            egui::RichText::new("\u{1f5b1} Mouse Tracking")
                .strong()
                .size(14.0),
        );
        ui.checkbox(
            &mut self.config.parallax.mouse.global_tracking,
            "Global mouse tracking (aggregated)",
        ).on_hover_text("Track the cursor using hyprctl/wlrctl (works across all windows).\nRequires hyprctl (Hyprland) or wlrctl (wlroots) installed.");
        ui.checkbox(
            &mut self.config.parallax.mouse.per_output_tracking,
            "Per-output mouse tracking (wl_pointer)",
        ).on_hover_text("Track the cursor per-monitor using wl_pointer events.\nWorks only while cursor is over the cava-bg surface.");
        ui.small(
            "Tip: For mouse/audio reaction, enable the option per-layer under 'Audio + Mouse Reaction'.",
        );

        // Profile-mode: save button + no default layer editing
        let has_profile = !self.selected_profile.is_empty();
        if has_profile {
            ui.separator();
            if ui
                .button("󰏆 Save to profile")
                .on_hover_text("Write the current layer configuration back to the profile file.")
                .clicked()
            {
                let dir = if self.profiles_dir_input.trim().is_empty() {
                    self.config_path
                        .parent()
                        .unwrap_or(Path::new(""))
                        .join("parallax")
                } else {
                    PathBuf::from(self.profiles_dir_input.trim())
                };
                match ParallaxProfile::load(&dir, &self.selected_profile) {
                    Ok(mut profile) => {
                        // Sync order + config from GUI layers into profile
                        profile.layers.clear();
                        for layer in &self.config.parallax.layers {
                            let source = std::path::Path::new(&layer.source);
                            if let Some(filename) = source.file_name().and_then(|s| s.to_str()) {
                                profile.layers.push(filename.to_string());
                                profile.update_layer_config(filename, layer.clone());
                            }
                        }
                        match profile.save(&dir) {
                            Ok(_) => {
                                self.status = format!("Saved profile '{}'", &self.selected_profile)
                            }
                            Err(e) => self.status = format!("Failed to save profile: {e}"),
                        }
                    }
                    Err(e) => self.status = format!("Failed to load profile for saving: {e}"),
                }
            }
            ui.small("Settings above are shared. Edit individual layers in the layer list below.");
        } else {
            ui.separator();
            ui.heading("Default Layer Configuration");
            ui.label("No active profile — configure layers manually:");
        }

        // Always show the common settings: mode, depth, draw, mouse (already shown above)
        ui.separator();
        ui.heading("Layer Auto-Detection");
        path_picker_row(
            ui,
            "Parallax layers directory",
            &mut self.layers_dir_input,
            true,
        );
        if ui.button("Scan layers directory")
            .on_hover_text("Auto-discover image files in the specified directory and add them as parallax layers.")
            .clicked() {
            let dir = self.layers_dir_input.trim();
            if dir.is_empty() {
                self.status = "Set a layers directory first.".to_string();
            } else {
                let path = std::path::Path::new(dir);
                if path.exists() {
                    let discovered =
                        crate::layer_finder::discover_parallax_layers(path);
                    if discovered.is_empty() {
                        self.status = format!(
                            "No supported layer files found in {}",
                            dir
                        );
                    } else {
                        let count = discovered.len();
                        for dl in &discovered {
                            let path_str =
                                dl.path.to_string_lossy().to_string();
                            if !self
                                .config
                                .parallax
                                .layers
                                .iter()
                                .any(|l| l.source == path_str)
                            {
                                let mut layer = default_parallax_layer();
                                layer.source = path_str;
                                layer.depth = dl.inferred_depth;
                                self.config.parallax.layers.push(layer);
                            }
                        }
                        self.status = format!(
                            "Discovered {} new layer(s)",
                            count
                        );
                    }
                } else {
                    self.status = format!("Directory not found: {}", dir);
                }
            }
        }

        ui.separator();
        ui.label(
            egui::RichText::new("\u{1f5bc} Layer Stack")
                .strong()
                .size(14.0),
        );
        let layers_len = self.config.parallax.layers.len();
        if layers_len == 0 && !has_profile {
            ui.label("No layers configured yet. Use the Scan button or add layers manually below.");
        }

        let mut remove_layer: Option<usize> = None;
        let mut move_up_layer: Option<usize> = None;
        let mut move_down_layer: Option<usize> = None;
        let layers_len = self.config.parallax.layers.len();

        // Show Cava Visualizer at its specified position among regular layers
        // by inserting it into the layer loop when we reach its index
        let mut rendered_viz = false;

        for (idx, layer) in self.config.parallax.layers.iter_mut().enumerate() {
            // Insert visualizer at the correct position
            if !rendered_viz
                && self.config.parallax.enabled
                && self.config.parallax.visualizer_as_parallax_layer
                && self.config.parallax.visualizer_layer_index <= idx
            {
                let viz_idx = self.config.parallax.visualizer_layer_index;
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong("\u{1f3b5} Cava Visualizer");
                        ui.small("(built-in visualizer)");
                        if viz_idx > 0 && ui.button("\u{2b06}").clicked() {
                            self.config.parallax.visualizer_layer_index = viz_idx.saturating_sub(1);
                        }
                        if viz_idx < layers_len && ui.button("\u{2b07}").clicked() {
                            self.config.parallax.visualizer_layer_index = viz_idx + 1;
                        }
                    });
                    ui.small("This layer is tied to the audio visualizer and cannot be removed.");
                });
                rendered_viz = true;
            }

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let label = if layer.name.is_empty() {
                        format!("Layer {}", idx + 1)
                    } else {
                        layer.name.clone()
                    };
                    ui.strong(label);
                    ui.text_edit_singleline(&mut layer.name);
                    if idx > 0
                        && ui.button("\u{2b06}").clicked() {
                            move_up_layer = Some(idx);
                        }
                    if idx + 1 < layers_len
                        && ui.button("\u{2b07}").clicked() {
                            move_down_layer = Some(idx);
                        }
                    if ui.button("\u{00d7} Remove").clicked() {
                        remove_layer = Some(idx);
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Source:")
                        .on_hover_text("File path to the source image for this parallax layer.");
                    ui.text_edit_singleline(&mut layer.source);
                    if ui.button("Browse")
                        .on_hover_text("Select an image file for this layer")
                        .clicked() {
                        let result = Arc::new(Mutex::new(None::<String>));
                        let result_clone = result.clone();
                        thread::spawn(move || {
                            if let Some(p) = FileDialog::new().pick_file() {
                                let mut guard = result_clone.lock().unwrap();
                                *guard = Some(p.to_string_lossy().to_string());
                            }
                        });
                        self.pending_file_result = Some(result);
                    }
                });

                let layer_type = layer.layer_type.get_or_insert(LayerSourceType::StaticImage);
                ui.horizontal(|ui| {
                    ui.label("Source Type:")
                        .on_hover_text("Type of the layer source:\n\nStatic Image — A fixed image file\nAnimated Image — GIF / animated PNG");
                    layer_source_type_combo(ui, layer_type, &format!("layer_type_{idx}"));
                });

                ui.horizontal(|ui| {
                    ui.label("Z Index:")
                        .on_hover_text("Depth ordering of this layer.\nHigher values = closer to viewer / drawn on top.");
                    ui.add(egui::DragValue::new(&mut layer.z_index))
                        .on_hover_text("Depth position. Higher = on top.");
                    ui.label("Opacity:")
                        .on_hover_text("Layer transparency. 1.0 = fully opaque, 0.0 = fully transparent.");
                    ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0));
                });

                ui.horizontal(|ui| {
                    ui.label("Blend Mode:")
                        .on_hover_text("How this layer blends with the layers below it.");
                    blend_mode_combo(ui, &mut layer.blend_mode, &format!("blend_mode_{idx}"));
                });

                ui.horizontal(|ui| {
                    ui.label("Offset X/Y:")
                        .on_hover_text("Position offset from the layer's original location.\nX = horizontal, Y = vertical.");
                    ui.add(egui::DragValue::new(&mut layer.offset[0]).speed(0.1))
                        .on_hover_text("Horizontal offset (pixels)");
                    ui.add(egui::DragValue::new(&mut layer.offset[1]).speed(0.1))
                        .on_hover_text("Vertical offset (pixels)");
                });
                ui.add(
                    egui::Slider::new(&mut layer.parallax_speed, 0.0..=4.0).text("Parallax Speed"),
                ).on_hover_text("Movement speed multiplier for this layer.\n0 = static, 1 = normal parallax, higher = faster drift.\nCombine with depth for realistic 3D effect.");

                ui.separator();
                ui.collapsing(format!("\u{266a} Audio + Mouse Reaction##{idx}"), |ui| {
                    ui.checkbox(&mut layer.react_to_audio, "React to Audio")
                        .on_hover_text("This layer moves in response to audio amplitude.\nUseful for subtle breathing/beating motion.");
                    if layer.react_to_audio {
                        ui.add(
                            egui::Slider::new(&mut layer.audio_reaction_intensity, 0.0..=2.0)
                                .text("Audio Sensitivity"),
                        ).on_hover_text("Strength of the audio reaction.\nHigher = layer moves more with the beat.");
                    }
                    ui.checkbox(&mut layer.react_to_mouse, "React to Mouse")
                        .on_hover_text("This layer shifts in response to cursor position.\nCreates a parallax depth effect where layers at different depths move at different speeds.");
                    if layer.react_to_mouse {
                        ui.add(
                            egui::Slider::new(&mut layer.mouse_depth_factor, 0.0..=2.0)
                                .text("Mouse Depth Factor"),
                        ).on_hover_text("How much this layer responds to mouse movement.\nHigher values = more displacement for the same cursor move.");
                    }
                });

                ui.collapsing(format!("\u{25b6} Animation##{idx}"), |ui| {
                    let anim = layer.animation.get_or_insert(LayerAnimationConfig {
                        enabled: false,
                        animation_type: AnimationType::Float,
                        speed: 1.0,
                        amplitude: 0.2,
                    });
                    ui.checkbox(&mut anim.enabled, "Enable Animation")
                        .on_hover_text("Apply a periodic animation to this layer (float, pulse, or bounce).");
                    if anim.enabled {
                        animation_type_combo(
                            ui,
                            &mut anim.animation_type,
                            &format!("anim_type_{idx}"),
                        );
                        ui.add(
                            egui::Slider::new(&mut anim.speed, 0.1..=10.0).text("Animation Speed"),
                        ).on_hover_text("How fast the animation cycles.\n1.0 = normal speed, higher = faster.");
                        ui.add(
                            egui::Slider::new(&mut anim.amplitude, 0.0..=2.0)
                                .text("Animation Amplitude"),
                        ).on_hover_text("How much the layer moves during animation.\nHigher values = more dramatic motion.");
                    }
                });

                ui.collapsing(format!("\u{2726} Drop Shadow##{idx}"), |ui| {
                    let shadow =
                        layer
                            .drop_shadow
                            .get_or_insert(crate::app_config::DropShadowConfig {
                                color: [0.0, 0.0, 0.0, 0.6],
                                offset: [8.0, 8.0],
                                blur_radius: 18.0,
                                spread: 0.0,
                            });
                    color_picker_row(ui, "Shadow Color", &mut shadow.color);
                    ui.horizontal(|ui| {
                        ui.label("Offset:")
                            .on_hover_text("Shadow position offset from the layer.\nX = horizontal shift, Y = vertical shift.");
                        ui.add(egui::DragValue::new(&mut shadow.offset[0]).speed(0.5))
                            .on_hover_text("Horizontal shadow offset");
                        ui.add(egui::DragValue::new(&mut shadow.offset[1]).speed(0.5))
                            .on_hover_text("Vertical shadow offset");
                    });
                    ui.add(egui::Slider::new(&mut shadow.blur_radius, 0.0..=80.0).text("Blur"))
                        .on_hover_text("How blurry the shadow edges are.\n0 = sharp, higher = softer.");
                    ui.add(egui::Slider::new(&mut shadow.spread, 0.0..=20.0).text("Spread"))
                        .on_hover_text("How far the shadow expands beyond the layer edges.");
                });
            });
            ui.separator();
        }

        if let Some(idx) = remove_layer {
            self.config.parallax.layers.remove(idx);
            ui.ctx().request_repaint();
        }
        if let Some(idx) = move_up_layer {
            if idx > 0 {
                self.config.parallax.layers.swap(idx, idx - 1);
                ui.ctx().request_repaint();
            }
        }
        if let Some(idx) = move_down_layer {
            if idx + 1 < self.config.parallax.layers.len() {
                self.config.parallax.layers.swap(idx, idx + 1);
                ui.ctx().request_repaint();
            }
        }

        if !has_profile
            && ui.button("+ Add Layer")
                .on_hover_text("Add a new empty parallax layer.\nConfigure its source image, depth, and behavior below.")
                .clicked() {
                self.config.parallax.layers.push(default_parallax_layer());
            }

        // If visualizer wasn't inserted in the loop (index >= layers.len()), show it at the end
        if !rendered_viz
            && self.config.parallax.enabled
            && self.config.parallax.visualizer_as_parallax_layer
        {
            let viz_idx = self.config.parallax.visualizer_layer_index;
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong("\u{1f3b5} Cava Visualizer");
                    ui.small("(built-in visualizer)");
                    if viz_idx > 0 && ui.button("\u{2b06}").clicked() {
                        self.config.parallax.visualizer_layer_index = viz_idx.saturating_sub(1);
                    }
                    if viz_idx < layers_len && ui.button("\u{2b07}").clicked() {
                        self.config.parallax.visualizer_layer_index = viz_idx + 1;
                    }
                });
                ui.small("This layer is tied to the audio visualizer and cannot be removed.");
            });
        }
    }

    fn section_xray(&mut self, ui: &mut egui::Ui) {
        ui.heading("□ X-Ray — Hidden Image Overlay");

        // ── Enable / disable ──────────────────────────────────────
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.xray.enabled, "Enable X-Ray")
                .on_hover_text("Overlay a hidden image that appears through the audio bars.\nThe bars act as a mask revealing the image behind them.");
            if ui.button("Save").on_hover_text("Apply changes immediately").clicked() {
                if let Err(e) = self.persist() {
                    self.status = format!("Save error: {}", e);
                }
            }
        });

        if !self.config.xray.enabled {
            ui.label("ℹ Disabled — audio bars render normally without hidden image.");
            return;
        }

        ui.separator();

        // ── Image source ─────────────────────────────────────
        ui.label(egui::RichText::new("🖼 Image Source").strong().size(14.0));
        let cfg = self
            .config
            .hidden_image
            .get_or_insert_with(Default::default);

        ui.horizontal(|ui| {
            ui.radio_value(&mut cfg.use_wallpaper, true, "From wallpaper directory");
            ui.radio_value(&mut cfg.use_wallpaper, false, "Pick a specific file");
        });

        if cfg.use_wallpaper {
            path_picker_row(ui, "Image directory", &mut self.xray_dir_input, true);
            ui.label("Images named after your current wallpaper will be auto-selected.")
                .on_hover_text("Place your x-ray images in this directory with the same base name\nas the wallpaper file. Example: wallpaper.jpg → wallpaper.png");
        }
        if !cfg.use_wallpaper {
            ui.horizontal(|ui| {
                ui.label("Image:");
                let path_str = cfg.path.get_or_insert_with(String::new);
                if ui
                    .button("Browse")
                    .on_hover_text("Select an image file from your system")
                    .clicked()
                {
                    let result = Arc::new(Mutex::new(None::<String>));
                    let result_clone = result.clone();
                    thread::spawn(move || {
                        if let Some(p) = FileDialog::new().pick_file() {
                            *result_clone.lock().unwrap() = Some(p.to_string_lossy().to_string());
                        }
                    });
                    self.pending_file_result = Some(result);
                    self.pending_file_for_xray = true;
                }
                ui.text_edit_singleline(path_str);
            });
        }

        ui.separator();

        // ── Visual effects ───────────────────────────────────
        ui.label(egui::RichText::new("✨ Image Effects").strong().size(14.0));

        ui.horizontal(|ui| {
            ui.label("Color effect:").on_hover_text(
                "Apply a color filter to the hidden image before blending:

None — Original colors
Grayscale — Black and white
Invert — Inverted colors
Sepia — Warm brownish tone",
            );
            let effect = &mut cfg.effect;
            let effect_label = match effect {
                crate::app_config::HiddenImageEffect::None => "None",
                crate::app_config::HiddenImageEffect::Grayscale => "Grayscale",
                crate::app_config::HiddenImageEffect::Invert => "Invert",
                crate::app_config::HiddenImageEffect::Sepia => "Sepia",
                crate::app_config::HiddenImageEffect::Palette(_) => "Palette",
            };
            egui::ComboBox::from_id_source("hidden_effect")
                .selected_text(effect_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(effect, crate::app_config::HiddenImageEffect::None, "None");
                    ui.selectable_value(
                        effect,
                        crate::app_config::HiddenImageEffect::Grayscale,
                        "Grayscale",
                    );
                    ui.selectable_value(
                        effect,
                        crate::app_config::HiddenImageEffect::Invert,
                        "Invert",
                    );
                    ui.selectable_value(
                        effect,
                        crate::app_config::HiddenImageEffect::Sepia,
                        "Sepia",
                    );
                });
        });

        ui.horizontal(|ui| {
            ui.label("Blend mode:")
                .on_hover_text("How the hidden image merges with the audio bars below it.\nDifferent blend modes produce different visual effects.");
            blend_mode_combo(ui, &mut cfg.blend_mode, "hidden_blend");
        });
    }

    fn section_colors(&mut self, ui: &mut egui::Ui) {
        ui.heading("Colors & Visual Effects");
        ui.separator();

        // ── Palette: único editor de colores ──────────────────────────
        ui.label(egui::RichText::new("Color Palette").strong().size(14.0));
        ui.small("Add colors to the palette. 1 color = solid, 2+ colors = sequence. Toggle gradient below.");

        let mut to_remove = None;
        let can_remove = self.config.colors.palette.len() > 1;
        for (idx, color) in self.config.colors.palette.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut rgb = [
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                ];
                ui.label(format!("#{}:", idx + 1));
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    color[0] = rgb[0] as f32 / 255.0;
                    color[1] = rgb[1] as f32 / 255.0;
                    color[2] = rgb[2] as f32 / 255.0;
                }
                ui.label("α:");
                ui.add(egui::Slider::new(&mut color[3], 0.0..=1.0).fixed_decimals(2));
                if can_remove && ui.button("×").on_hover_text("Remove").clicked() {
                    to_remove = Some(idx);
                }
            });
        }
        if let Some(idx) = to_remove {
            self.config.colors.palette.remove(idx);
        }
        ui.horizontal(|ui| {
            if ui.button("+ Add Color").clicked() {
                self.config.colors.palette.push([1.0, 0.0, 1.0, 1.0]);
            }
            if self.config.colors.palette.len() > 1 {
                if ui
                    .checkbox(&mut self.config.colors.use_gradient, "🌈 Use gradient")
                    .changed()
                {
                    // use_gradient field toggles between bar sequence and gradient interpolation
                }
                if self.config.colors.palette.len() > 1 && self.config.colors.use_gradient {
                    ui.horizontal(|ui| {
                        ui.label("Direction:");
                        egui::ComboBox::from_id_source("grad_dir")
                            .selected_text(match self.config.colors.gradient_direction {
                                crate::app_config::GradientDirection::BottomToTop => "Bottom → Top",
                                crate::app_config::GradientDirection::TopToBottom => "Top → Bottom",
                                crate::app_config::GradientDirection::LeftToRight => "Left → Right",
                                crate::app_config::GradientDirection::RightToLeft => "Right → Left",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.config.colors.gradient_direction,
                                    crate::app_config::GradientDirection::BottomToTop,
                                    "Bottom → Top",
                                );
                                ui.selectable_value(
                                    &mut self.config.colors.gradient_direction,
                                    crate::app_config::GradientDirection::TopToBottom,
                                    "Top → Bottom",
                                );
                                ui.selectable_value(
                                    &mut self.config.colors.gradient_direction,
                                    crate::app_config::GradientDirection::LeftToRight,
                                    "Left → Right",
                                );
                                ui.selectable_value(
                                    &mut self.config.colors.gradient_direction,
                                    crate::app_config::GradientDirection::RightToLeft,
                                    "Right → Left",
                                );
                            });
                    });
                }
            }
        });

        // ── Gradient preview ──────────────────────────────────────────
        if self.config.colors.palette.len() > 1 && self.config.colors.use_gradient {
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
            draw_gradient_preview(
                ui.painter(),
                rect,
                &self.config.colors.palette,
                self.config.colors.gradient_direction,
            );
        }

        ui.separator();

        // ── Bar alpha ────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.label("Bar Alpha:");
            ui.add(
                egui::Slider::new(&mut self.config.audio.bar_alpha, 0.0..=1.0).text("transparency"),
            );
        });

        // ── Wallpaper extraction ──────────────────────────────────────
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.config.colors.extract_from_wallpaper,
                "🎨 Auto-extract from wallpaper",
            );
            if self.config.colors.extract_from_wallpaper {
                if ui.button("Extract Now").clicked() {
                    match WallpaperAnalyzer::find_wallpaper() {
                        Some(wallpaper_path) => {
                            let requested = self.config.colors.palette.len().max(2);
                            match WallpaperAnalyzer::extract_colors(
                                &wallpaper_path,
                                self.config.colors.extraction_mode,
                                requested,
                            ) {
                                Ok(extracted) => {
                                    let count = extracted.len();
                                    self.config.colors.palette = extracted;
                                    match self.persist() {
                                        Ok(_) => {
                                            self.status =
                                                format!("Extracted {count} colors from wallpaper")
                                        }
                                        Err(e) => {
                                            self.status = format!(
                                                "Extracted {count} colors, but persist failed: {e}"
                                            )
                                        }
                                    }
                                }
                                Err(e) => self.status = format!("Extraction failed: {e}"),
                            }
                        }
                        None => self.status = "No wallpaper detected".to_string(),
                    }
                }
                extraction_mode_combo(
                    ui,
                    &mut self.config.colors.extraction_mode,
                    "colors_extraction_mode",
                );
            }
        });

        // ── Dynamic colors toggle ────────────────────────────────────
        ui.checkbox(
            &mut self.config.general.dynamic_colors,
            "Dynamic colors (from network)",
        );
    }

    fn section_performance(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Frame rate limit");
            ui.add(egui::DragValue::new(&mut self.config.advanced.frame_rate_limit).speed(1));
        });
        ui.horizontal(|ui| {
            ui.label("Layer cache size");
            ui.add(egui::DragValue::new(&mut self.config.advanced.layer_cache_size).speed(1));
        });
        ui.checkbox(&mut self.config.performance.vsync, "VSync");
        ui.checkbox(
            &mut self.config.performance.multi_threaded_decode,
            "Multi-threaded decode",
        );

        ui.separator();
        ui.collapsing("Idle mode", |ui| {
            ui.checkbox(
                &mut self.config.performance.idle_mode.enabled,
                "Enable idle mode",
            );
            ui.add_enabled_ui(self.config.performance.idle_mode.enabled, |ui| {
                ui.add(
                    egui::Slider::new(
                        &mut self.config.performance.idle_mode.audio_threshold,
                        0.0..=0.15,
                    )
                    .text("Audio threshold"),
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.config.performance.idle_mode.timeout_seconds,
                        0.5..=30.0,
                    )
                    .text("Silence timeout (s)"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.performance.idle_mode.idle_fps, 1..=30)
                        .text("Idle FPS"),
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.config.performance.idle_mode.exit_transition_ms,
                        0..=1500,
                    )
                    .text("Exit transition (ms)"),
                );
            });
        });

        ui.separator();
        ui.collapsing("Video decoder", |ui| {
            ui.checkbox(
                &mut self.config.performance.video_decoder.lazy_init,
                "Lazy init",
            );
            ui.checkbox(
                &mut self.config.performance.video_decoder.auto_shutdown,
                "Auto shutdown when unused",
            );
            ui.add_enabled_ui(self.config.performance.video_decoder.auto_shutdown, |ui| {
                ui.add(
                    egui::Slider::new(
                        &mut self.config.performance.video_decoder.shutdown_after_seconds,
                        2.0..=300.0,
                    )
                    .text("Shutdown timeout (s)"),
                );
            });
            ui.checkbox(
                &mut self.config.performance.video_decoder.pause_on_idle,
                "Pause decoder while idle",
            );
            ui.checkbox(
                &mut self.config.performance.video_decoder.debug_telemetry,
                "Decoder telemetry (debug)",
            );
        });

        ui.separator();
        ui.collapsing("X-Ray texture optimization", |ui| {
            ui.add(
                egui::Slider::new(
                    &mut self.config.performance.xray.prescale_max_dimension,
                    512..=4096,
                )
                .text("Pre-scale max dimension"),
            );
            ui.checkbox(
                &mut self.config.performance.xray.generate_mipmaps,
                "Generate mipmaps",
            );
            egui::ComboBox::from_id_source("xray_mask_compute_mode")
                .selected_text(format!(
                    "{:?}",
                    self.config.performance.xray.mask_compute_mode
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.performance.xray.mask_compute_mode,
                        MaskComputeMode::Auto,
                        "Auto",
                    );
                    ui.selectable_value(
                        &mut self.config.performance.xray.mask_compute_mode,
                        MaskComputeMode::Cpu,
                        "CPU",
                    );
                    ui.selectable_value(
                        &mut self.config.performance.xray.mask_compute_mode,
                        MaskComputeMode::Gpu,
                        "GPU (fallback to CPU)",
                    );
                });
        });

        ui.separator();
        ui.collapsing("Performance metrics", |ui| {
            ui.checkbox(
                &mut self.config.performance.telemetry.enabled,
                "Enable runtime metrics (debug mode)",
            );
            ui.add(
                egui::Slider::new(
                    &mut self.config.performance.telemetry.metrics_window,
                    32..=1000,
                )
                .text("Metrics window"),
            );
            ui.add(
                egui::Slider::new(
                    &mut self.config.performance.telemetry.log_interval_seconds,
                    1..=30,
                )
                .text("Log interval (s)"),
            );
            ui.small("Metrics are emitted only when debug/verbose mode is enabled.");
        });
    }

    fn section_advanced(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.config.advanced.verbose_logging, "Verbose logging");
        ui.checkbox(
            &mut self.config.general.disable_audio,
            "Disable audio input",
        );
        ui.horizontal(|ui| {
            ui.label("Target output");

        ui.horizontal(|ui| {
            ui.label("Target outputs (select one or more):");
            if ui.button("Refresh outputs").clicked() {
                self.detected_outputs = load_runtime_output_names(&self.config_path);
                if self.detected_outputs.is_empty() {
                    self.status =
                        "No runtime outputs found yet. Start daemon and try again.".to_string();
                }
            }
        });

        let all_selected = self.config.general.preferred_outputs.is_empty();
        if ui
            .checkbox(&mut false, "All outputs")
            .on_hover_text("Clear selection to render on all outputs")
            .clicked()
        {
            self.config.general.preferred_outputs.clear();
        }
        if !all_selected {
            for output in &self.detected_outputs.clone() {
                let mut selected = self.config.general.preferred_outputs.contains(output);
                let was = selected;
                ui.checkbox(&mut selected, output);
                if selected != was {
                    if selected {
                        self.config.general.preferred_outputs.push(output.clone());
                    } else {
                        self.config.general
                            .preferred_outputs
                            .retain(|v| v != output);
                    }
                }
            }
        } else {
            ui.small("Rendering on ALL detected outputs.");
        }

        ui.horizontal(|ui| {
            ui.label("Manual output pattern (comma-separated):");
            let mut manual = self.config.general.preferred_outputs.join(", ");
            if ui.text_edit_singleline(&mut manual).changed() {
                self.config.general.preferred_outputs = manual
                    .split([',', ';'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        });
        ui.small("Type connector names separated by commas (e.g. eDP-1, HDMI-A-1). Leave empty for all outputs.");
        });
        ui.small("You can type a connector name (example: eDP-1, HDMI-A-1) or choose from detected outputs.");

        ui.separator();
        ui.collapsing("Per-output overrides", |ui| {
            ui.small("Save the current editor state as an override for a specific output.");

            ui.horizontal(|ui| {
                ui.label("Output key");
                ui.text_edit_singleline(&mut self.selected_output_override);
                if ui.button("Create / update from current config").clicked() {
                    let key = self.selected_output_override.trim().to_string();
                    if key.is_empty() {
                        self.status = "Output key cannot be empty.".to_string();
                    } else {
                        let entry = self.config.output.entry(key.clone()).or_default();
                        entry.enabled = Some(true);
                        entry.name = Some(key.clone());
                        entry.config.general = Some(self.config.general.clone());
                        entry.config.audio = Some(self.config.audio.clone());
                        entry.config.colors = Some(self.config.colors.clone());
                        entry.config.display = Some(self.config.display.clone());
                        entry.config.smoothing = Some(self.config.smoothing.clone());
                        entry.config.hidden_image = self.config.hidden_image.clone();
                        entry.config.layers = self.config.layers.clone();
                        entry.config.parallax = Some(self.config.parallax.clone());
                        entry.config.wallpaper = Some(self.config.wallpaper.clone());
                        entry.config.xray_mask = Some(self.config.xray_mask.clone());
                        entry.config.xray = Some(self.config.xray.clone());
                        entry.config.performance = Some(self.config.performance.clone());
                        entry.config.advanced = Some(self.config.advanced.clone());
                        self.status = format!("Per-output override updated for {key}");
                    }
                }
            });

            if !self.config.output.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Existing overrides:");
                    for key in self.config.output.keys() {
                        if ui.button(key).clicked() {
                            self.selected_output_override = key.clone();
                        }
                    }
                });
            }

            if !self.selected_output_override.trim().is_empty() {
                let mut pending_copy: Option<(String, crate::app_config::OutputOverrideConfig)> =
                    None;

                if let Some(ovr) = self
                    .config
                    .output
                    .get_mut(self.selected_output_override.trim())
                {
                    ui.horizontal(|ui| {
                        let mut enabled = ovr.enabled.unwrap_or(true);
                        if ui.checkbox(&mut enabled, "Enabled").changed() {
                            ovr.enabled = Some(enabled);
                        }
                        ui.label("Pattern");
                        let name = ovr.name.get_or_insert_with(String::new);
                        ui.text_edit_singleline(name);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Copy selected override to");
                        ui.text_edit_singleline(&mut self.copy_output_target);
                        if ui.button("Copy").clicked() {
                            let target = self.copy_output_target.trim().to_string();
                            if target.is_empty() {
                                self.status = "Copy target cannot be empty.".to_string();
                            } else {
                                pending_copy = Some((target, ovr.clone()));
                            }
                        }
                    });
                }

                if let Some((target, snapshot)) = pending_copy {
                    self.config.output.insert(target.clone(), snapshot);
                    self.status = format!("Override copied to {target}");
                }
            }
        });

        ui.separator();
        ui.collapsing("Presets (TOML)", |ui| {
            ui.horizontal(|ui| {
                ui.label("Preset name");
                ui.text_edit_singleline(&mut self.preset_name_input);
                if ui.button("Save preset").clicked() {
                    let name = self.preset_name_input.trim().to_string();
                    match self.save_preset(&name) {
                        Ok(_) => self.status = format!("Preset saved: {name}"),
                        Err(e) => self.status = format!("Preset save error: {e}"),
                    }
                }
                if ui.button("Refresh").clicked() {
                    self.available_presets = list_presets();
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Available:");
                for name in self.available_presets.clone() {
                    if ui.button(format!("Load {name}")).clicked() {
                        match self.load_preset(&name) {
                            Ok(_) => self.status = format!("Preset loaded: {name}"),
                            Err(e) => self.status = format!("Preset load error: {e}"),
                        }
                    }
                }
            });

            ui.horizontal(|ui| {
                if ui.button("Import preset (.toml)").clicked() {
                    if let Some(path) = FileDialog::new().add_filter("toml", &["toml"]).pick_file()
                    {
                        match fs::read_to_string(&path)
                            .ok()
                            .and_then(|s| toml::from_str::<Config>(&s).ok())
                        {
                            Some(mut cfg) => {
                                cfg.normalize_compat_fields();
                                self.config = cfg;
                                self.status = format!("Imported preset from {}", path.display());
                            }
                            None => {
                                self.status = format!("Invalid preset file: {}", path.display());
                            }
                        }
                    }
                }
                if ui.button("Export current preset").clicked() {
                    if let Some(path) = FileDialog::new()
                        .set_file_name("cava-bg-preset.toml")
                        .save_file()
                    {
                        match toml::to_string_pretty(&self.config)
                            .ok()
                            .and_then(|s| fs::write(&path, s).ok())
                        {
                            Some(_) => {
                                self.status = format!("Preset exported to {}", path.display());
                            }
                            None => {
                                self.status =
                                    format!("Failed to export preset to {}", path.display());
                            }
                        }
                    }
                }
            });
        });

        ui.label("Custom shader path: currently handled by build-time include (advanced runtime override pending).");
    }

    fn daemon_panel(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Daemon Control").strong());
            self.refresh_daemon_status();

            let (dot, text) = if self.daemon_status.running {
                (
                    RichText::new("●").color(Color32::LIGHT_GREEN),
                    format!(
                        "Running (PID: {})",
                        self.daemon_status.pid.unwrap_or_default()
                    ),
                )
            } else {
                (
                    RichText::new("●").color(Color32::LIGHT_RED),
                    "Stopped".to_string(),
                )
            };

            ui.horizontal(|ui| {
                ui.label(dot);
                ui.label(text);
            });
            ui.small(format!(
                "PID file: {}",
                self.daemon_status.pid_file.display()
            ));

            ui.horizontal(|ui| {
                if ui.button("Start").clicked() {
                    match self.execute_daemon_command("on") {
                        Ok(msg) => self.status = msg,
                        Err(e) => self.status = format!("Start error: {e}"),
                    }
                }
                if ui.button("Stop").clicked() {
                    match self.execute_daemon_command("off") {
                        Ok(msg) => self.status = msg,
                        Err(e) => self.status = format!("Stop error: {e}"),
                    }
                }
                if ui.button("Restart").clicked() {
                    match self.restart_daemon() {
                        Ok(_) => {}
                        Err(e) => self.status = format!("Restart error: {e}"),
                    }
                }
            });
        });
    }

    fn footer_buttons(&mut self, ui: &mut egui::Ui, has_errors: bool) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!has_errors, egui::Button::new("Save Config (Ctrl+S)"))
                .clicked()
            {
                match self.persist() {
                    Ok(_) => self.status = "Configuration saved.".to_string(),
                    Err(e) => self.status = format!("Save error: {e}"),
                }
            }

            if ui
                .add_enabled(
                    !has_errors,
                    egui::Button::new("[OK] Apply (save + restart daemon)"),
                )
                .clicked()
            {
                self.show_apply_confirm = true;
            }

            if ui.button("↺ Reset Defaults").clicked() {
                self.show_reset_confirm = true;
            }
        });
    }

    fn confirmation_dialogs(&mut self, ctx: &egui::Context) {
        if self.show_apply_confirm {
            egui::Window::new("Confirm Apply")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Apply will save changes and notify renderer via hot-reload. Continue?",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Yes, apply now").clicked() {
                            match self.persist() {
                                Ok(_) => {
                                    self.status = "Config saved.".to_string();
                                    self.refresh_daemon_status();
                                    if self.daemon_status.running {
                                        match self.restart_daemon() {
                                            Ok(_) => {}
                                            Err(e) => {
                                                self.status = format!("Apply + restart error: {e}");
                                            }
                                        }
                                    } else {
                                        self.status = "Configuration saved. Daemon is not running — start it to see changes.".to_string();
                                    }
                                }
                                Err(e) => {
                                    self.status = format!("Apply error: {e}");
                                }
                            }
                            self.show_apply_confirm = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_apply_confirm = false;
                        }
                    });
                });
        }

        if self.show_reset_confirm {
            egui::Window::new("Confirm Reset")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Restore all values to defaults?");
                    ui.horizontal(|ui| {
                        if ui.button("Reset").clicked() {
                            self.reset_defaults();
                            self.show_reset_confirm = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_reset_confirm = false;
                        }
                    });
                });
        }
    }

    fn describe_visualization_mode(&self) -> &'static str {
        match self.config.audio.visualization_mode {
            VisualizationMode::Bars => "\u{25A0} Standard equalizer bars rising from bottom.",
            VisualizationMode::MirrorBars => {
                "\u{25A0} Bars mirrored vertically from center. Symmetric DJ-style."
            }
            VisualizationMode::InvertedBars => "\u{25A0} Bars hanging from the top edge.",
            VisualizationMode::Blocks => "\u{25A0} Pixelated block-style bars. Chunky and retro.",
            VisualizationMode::Waveform => {
                "\u{223F} Connected line tracing audio amplitude over time."
            }
            VisualizationMode::Spectrum => "\u{223F} Smooth line connecting frequency band peaks.",
            VisualizationMode::Ring => {
                "\u{25C9} Ring whose thickness varies with frequency amplitude."
            }
        }
    }

    fn describe_bar_shape(&self) -> &'static str {
        match self.config.audio.bar_shape {
            BarShape::Rectangle => "\u{25A0} Sharp-edged rectangular bars.",
            BarShape::Circle => "\u{25CF} Round dots instead of rectangles. Softer look.",
            BarShape::Triangle => "\u{25B2} Triangular bars pointing upward.",
            BarShape::Line => "\u{2015} Thin horizontal lines instead of bars.",
        }
    }
}

impl eframe::App for ConfigEditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Self::apply_visuals(ctx);
        self.handle_save_shortcut(ctx);

        // Check for pending file dialog result
        if let Some(path) = self.check_pending_file() {
            self.status = format!("Selected file: {}", path);

            if self.pending_file_for_xray {
                // Assign to hidden image path (Xray direct image)
                let cfg = self
                    .config
                    .hidden_image
                    .get_or_insert_with(Default::default);
                cfg.path = Some(path);
                self.pending_file_for_xray = false;
            } else {
                // Assign to parallax layer source
                let mut assigned = false;
                for layer in &mut self.config.parallax.layers {
                    if layer.source.is_empty() {
                        layer.source = path.clone();
                        assigned = true;
                        break;
                    }
                }
                if !assigned {
                    // Create a new layer with this source
                    let mut new_layer = crate::app_config::ParallaxLayerConfig::default();
                    new_layer.source = path.clone();
                    self.config.parallax.layers.push(new_layer);
                }
            }
        }

        self.sync_inputs_into_config();
        let errors = self.validate();

        egui::CentralPanel::default().show(ctx, |ui| {
            self.header(ui);
            self.tabs(ui);

            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                ConfigTab::Audio => self.section_audio(ui),
                ConfigTab::Visualizer => self.section_visualizer(ui),
                ConfigTab::Effects => self.section_effects(ui),
                ConfigTab::Colors => self.section_colors(ui),
                ConfigTab::Xray => self.section_xray(ui),
                ConfigTab::Parallax => self.section_layers_parallax(ui),
                ConfigTab::Performance => self.section_performance(ui),
                ConfigTab::Advanced => self.section_advanced(ui),
            });

            ui.add_space(8.0);
            self.daemon_panel(ui);

            if !errors.is_empty() {
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.label(
                        RichText::new("Validation issues")
                            .color(Color32::YELLOW)
                            .strong(),
                    );
                    for err in &errors {
                        ui.label(format!("• {err}"));
                    }
                });
            }

            ui.add_space(10.0);
            self.footer_buttons(ui, !errors.is_empty());
            ui.separator();
            ui.label(format!("Status: {}", self.status));
        });

        if errors.is_empty() {
            self.maybe_push_live_update();
        }

        self.confirmation_dialogs(ctx);
    }
}

fn draw_gradient_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    colors: &[[f32; 4]],
    direction: crate::app_config::GradientDirection,
) {
    use crate::app_config::GradientDirection;

    if colors.is_empty() {
        painter.rect_filled(rect, 4.0, Color32::BLACK);
        return;
    }

    if colors.len() == 1 {
        let c = color32_from_rgba(colors[0]);
        painter.rect_filled(rect, 4.0, c);
        return;
    }

    let steps = 120;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let t_mid = (t0 + t1) * 0.5;
        let rgba = sample_gradient_color(colors, t_mid);
        let c = color32_from_rgba(rgba);

        let segment = match direction {
            GradientDirection::LeftToRight => egui::Rect::from_min_max(
                egui::pos2(rect.left() + rect.width() * t0, rect.top()),
                egui::pos2(rect.left() + rect.width() * t1, rect.bottom()),
            ),
            GradientDirection::RightToLeft => egui::Rect::from_min_max(
                egui::pos2(rect.left() + rect.width() * (1.0 - t1), rect.top()),
                egui::pos2(rect.left() + rect.width() * (1.0 - t0), rect.bottom()),
            ),
            GradientDirection::TopToBottom => egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + rect.height() * t0),
                egui::pos2(rect.right(), rect.top() + rect.height() * t1),
            ),
            GradientDirection::BottomToTop => egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + rect.height() * (1.0 - t1)),
                egui::pos2(rect.right(), rect.top() + rect.height() * (1.0 - t0)),
            ),
        };

        painter.rect_filled(segment, 0.0, c);
    }

    painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, Color32::DARK_GRAY));
}

fn sample_gradient_color(colors: &[[f32; 4]], t: f32) -> [f32; 4] {
    let clamped_t = t.clamp(0.0, 1.0);
    if colors.len() == 1 {
        return colors[0];
    }

    let scaled = clamped_t * (colors.len().saturating_sub(1)) as f32;
    let low_idx = scaled.floor() as usize;
    let high_idx = (low_idx + 1).min(colors.len() - 1);
    let local_t = (scaled - low_idx as f32).clamp(0.0, 1.0);

    let low = colors[low_idx];
    let high = colors[high_idx];

    [
        low[0] + (high[0] - low[0]) * local_t,
        low[1] + (high[1] - low[1]) * local_t,
        low[2] + (high[2] - low[2]) * local_t,
        low[3] + (high[3] - low[3]) * local_t,
    ]
}

fn color32_from_rgba(rgba: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgba[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn tab_button(ui: &mut egui::Ui, current: &mut ConfigTab, tab: ConfigTab, label: &str) {
    let selected = *current == tab;
    if ui.selectable_label(selected, label).clicked() {
        *current = tab;
    }
}

fn color_picker_row(ui: &mut egui::Ui, label: &str, color: &mut [f32; 4]) {
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.label(label);
        }
        let mut rgba = egui::Rgba::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
        egui::color_picker::color_edit_button_rgba(
            ui,
            &mut rgba,
            egui::color_picker::Alpha::BlendOrAdditive,
        );
        *color = [rgba.r(), rgba.g(), rgba.b(), rgba.a()];
    });
}

fn blend_mode_combo(ui: &mut egui::Ui, value: &mut BlendMode, _id: &str) {
    // Only Normal blend mode is functional — other modes are removed
    // as they don't produce visible effects in the current renderer.
    *value = BlendMode::Normal;
    ui.label("Normal");
}

fn bar_shape_combo(ui: &mut egui::Ui, value: &mut BarShape, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{:?}", value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, BarShape::Rectangle, "Rectangle");
            ui.selectable_value(value, BarShape::Circle, "Circle");
            ui.selectable_value(value, BarShape::Triangle, "Triangle");
            ui.selectable_value(value, BarShape::Line, "Line");
        });
}

fn visualization_mode_combo(ui: &mut egui::Ui, value: &mut VisualizationMode, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(viz_mode_label(*value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, VisualizationMode::Bars, "Bars");
            ui.selectable_value(value, VisualizationMode::MirrorBars, "Mirror Bars");
            ui.selectable_value(value, VisualizationMode::InvertedBars, "Inverted Bars");
            ui.selectable_value(value, VisualizationMode::Blocks, "Blocks");
            ui.selectable_value(value, VisualizationMode::Waveform, "Waveform");
            ui.selectable_value(value, VisualizationMode::Spectrum, "Spectrum");
            ui.selectable_value(value, VisualizationMode::Ring, "Ring");
        });
}

/// Friendly label for the current visualization mode (used in combo headers / status).
fn viz_mode_label(value: VisualizationMode) -> &'static str {
    match value {
        VisualizationMode::Bars => "Bars",
        VisualizationMode::MirrorBars => "Mirror Bars",
        VisualizationMode::InvertedBars => "Inverted Bars",
        VisualizationMode::Blocks => "Blocks",
        VisualizationMode::Waveform => "Waveform",
        VisualizationMode::Spectrum => "Spectrum",
        VisualizationMode::Ring => "Ring",
    }
}

fn extraction_mode_combo(ui: &mut egui::Ui, value: &mut ColorExtractionMode, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{:?}", value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, ColorExtractionMode::Dominant, "Dominant");
            ui.selectable_value(value, ColorExtractionMode::Vibrant, "Vibrant");
            ui.selectable_value(value, ColorExtractionMode::Palette, "Palette");
        });
}

fn parallax_mode_combo(ui: &mut egui::Ui, value: &mut ParallaxMode, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{:?}", value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, ParallaxMode::AudioReactive, "AudioReactive");
            ui.selectable_value(value, ParallaxMode::MouseReactive, "MouseReactive");
            ui.selectable_value(value, ParallaxMode::Animated, "Animated");
            ui.selectable_value(value, ParallaxMode::Hybrid, "Hybrid");
        });
}

fn animation_type_combo(ui: &mut egui::Ui, value: &mut AnimationType, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{:?}", value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, AnimationType::Float, "Float");
            ui.selectable_value(value, AnimationType::Rotate, "Rotate");
            ui.selectable_value(value, AnimationType::Scale, "Scale");
            ui.selectable_value(value, AnimationType::Pulse, "Pulse");
            ui.selectable_value(value, AnimationType::Wiggle, "Wiggle");
        });
}

fn layer_source_type_combo(ui: &mut egui::Ui, value: &mut LayerSourceType, id: &str) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{:?}", value))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, LayerSourceType::StaticImage, "Image");
            ui.selectable_value(value, LayerSourceType::Video, "Video");
            ui.selectable_value(value, LayerSourceType::Gif, "Gif");
        });
}

/// Helper: returns a &mut String from HiddenImageConfig.path for the path_picker_row widget.
#[allow(dead_code)]
fn path_buffer_for_hidden(cfg: &mut crate::app_config::HiddenImageConfig) -> &mut String {
    // We store the path in a fixed buffer for GUI editing; if None, create a default.
    cfg.path.get_or_insert_with(String::new)
}

fn path_picker_row(ui: &mut egui::Ui, label: &str, target: &mut String, folder: bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.button("Browse").clicked() {
            let chosen = if folder {
                FileDialog::new()
                    .pick_folder()
                    .map(|p| p.to_string_lossy().to_string())
            } else {
                FileDialog::new()
                    .pick_file()
                    .map(|p| p.to_string_lossy().to_string())
            };
            if let Some(path) = chosen {
                *target = path;
            }
        }
        ui.text_edit_singleline(target);
    });
}

fn preset_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("cava-bg")
        .join("presets")
}

fn list_presets() -> Vec<String> {
    let dir = preset_dir();
    if fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

fn process_exists(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn read_pid_from_file(path: &Path) -> Option<i32> {
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse::<i32>().ok()
}

fn default_parallax_layer() -> ParallaxLayerConfig {
    ParallaxLayerConfig {
        source: String::new(),
        layer_type: Some(LayerSourceType::StaticImage),
        blend_mode: BlendMode::Normal,
        ..ParallaxLayerConfig::default()
    }
}

fn ensure_layers_exist(config: &mut Config) {
    if config.layers.is_none() {
        config.layers = Some(crate::app_config::LayersConfig {
            base: crate::app_config::LayerConfig {
                enabled: true,
                source: crate::app_config::LayerSourceConfig {
                    r#type: LayerSourceType::StaticImage,
                    path: String::new(),
                    looping: true,
                },
                fit: "cover".to_string(),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                max_buffered_frames: 5,
                frame_cache_size: 120,
            },
            reveal: crate::app_config::LayerConfig {
                enabled: true,
                source: crate::app_config::LayerSourceConfig {
                    r#type: LayerSourceType::StaticImage,
                    path: String::new(),
                    looping: true,
                },
                fit: "cover".to_string(),
                opacity: 1.0,
                blend_mode: BlendMode::Reveal,
                max_buffered_frames: 5,
                frame_cache_size: 120,
            },
            sync: Default::default(),
        });
    }
}

fn infer_source_type(path: &str) -> LayerSourceType {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "gif" => LayerSourceType::Gif,
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" | "flv" | "wmv" => LayerSourceType::Video,
        _ => LayerSourceType::StaticImage,
    }
}
