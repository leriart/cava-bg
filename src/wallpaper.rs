use anyhow::{Context, Result};
use image::{GenericImageView, Pixel};
use log;
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use walkdir::WalkDir;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.5;

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    /// Encuentra el archivo de wallpaper actual. Devuelve Some si lo encuentra.
    pub fn find_wallpaper() -> Option<PathBuf> {
        // 1. mpvpaper (muy común para GIFs/videos)
        if let Some(path) = Self::from_mpvpaper() {
            log::info!("Detected wallpaper via mpvpaper: {:?}", path);
            return Some(path);
        }

        // 2. Preguntar a hyprpaper a través de hyprctl
        if let Some(path) = Self::from_hyprctl_hyprpaper() {
            log::info!("Detected wallpaper via hyprctl hyprpaper: {:?}", path);
            return Some(path);
        }

        // 3. awww / swww
        if let Some(path) = Self::from_swww_like("awww") {
            log::info!("Detected wallpaper via awww: {:?}", path);
            return Some(path);
        }
        if let Some(path) = Self::from_swww_like("swww") {
            log::info!("Detected wallpaper via swww: {:?}", path);
            return Some(path);
        }

        // 4. swaybg
        if let Some(path) = Self::from_swaybg() {
            log::info!("Detected wallpaper via swaybg: {:?}", path);
            return Some(path);
        }

        // 5. hyprpaper (archivo de configuración)
        if let Some(path) = Self::from_hyprpaper_conf() {
            log::info!("Detected wallpaper via hyprpaper.conf: {:?}", path);
            return Some(path);
        }

        // 6. wpaperd
        if let Some(path) = Self::from_wpaperd() {
            log::info!("Detected wallpaper via wpaperd: {:?}", path);
            return Some(path);
        }

        // 7. wbg
        if let Some(path) = Self::from_wbg() {
            log::info!("Detected wallpaper via wbg: {:?}", path);
            return Some(path);
        }

        // 8. waypaper (config)
        if let Some(path) = Self::from_waypaper() {
            log::info!("Detected wallpaper via waypaper config: {:?}", path);
            return Some(path);
        }

        // 9. Fallback: buscar la imagen más reciente en directorios típicos
        if let Some(path) = Self::fallback_most_recent() {
            log::info!("Fallback: using most recent image in wallpaper dirs: {:?}", path);
            return Some(path);
        }

        log::warn!("No wallpaper could be detected by any method");
        None
    }

    // --- Métodos concretos de detección ---

    fn from_hyprctl_hyprpaper() -> Option<PathBuf> {
        let output = Command::new("hyprctl")
            .args(["hyprpaper", "listloaded"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.starts_with("wallpaper ") {
                let path_str = line.trim_start_matches("wallpaper ").trim();
                let path = PathBuf::from(path_str);
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_swww_like(cmd: &str) -> Option<PathBuf> {
        let output = Command::new(cmd).arg("query").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some((_monitor, path_str)) = line.split_once(':') {
                let path = PathBuf::from(path_str.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_swaybg() -> Option<PathBuf> {
        let output = Command::new("ps").arg("aux").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("swaybg") && line.contains("-i") {
                if let Some(idx) = line.find("-i") {
                    let rest = &line[idx + 2..].trim();
                    if let Some(path_str) = rest.split_whitespace().next() {
                        let path = PathBuf::from(path_str);
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    fn from_hyprpaper_conf() -> Option<PathBuf> {
        let conf = dirs::config_dir()?.join("hypr/hyprpaper.conf");
        if !conf.exists() {
            return None;
        }
        let content = std::fs::read_to_string(conf).ok()?;
        for line in content.lines() {
            if line.starts_with("wallpaper") || line.starts_with("preload") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() >= 2 {
                    let right = parts[1].trim();
                    if let Some((_, path_str)) = right.split_once(',') {
                        let path = PathBuf::from(path_str);
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    fn from_mpvpaper() -> Option<PathBuf> {
        // Usamos ps aux para obtener la línea completa
        let output = Command::new("ps").arg("aux").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("mpvpaper") && !line.contains("grep") {
                log::debug!("Found mpvpaper process: {}", line);
                // Dividir por espacios y buscar el último argumento que sea una ruta de archivo válida
                let parts: Vec<&str> = line.split_whitespace().collect();
                // mpvpaper suele tener la ruta del archivo al final de la línea
                for part in parts.iter().rev() {
                    let path = PathBuf::from(part);
                    // Verificar si el archivo existe y tiene extensión de imagen/video
                    if path.exists() {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            let ext_lower = ext.to_lowercase();
                            if matches!(ext_lower.as_str(),
                                "jpg" | "jpeg" | "png" | "bmp" | "webp" | "gif" |
                                "mp4" | "mkv" | "webm" | "avi" | "mov")
                            {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn from_wpaperd() -> Option<PathBuf> {
        let state_file = dirs::runtime_dir()?.join("wpaperd.state");
        if state_file.exists() {
            if let Ok(content) = std::fs::read_to_string(state_file) {
                let path = PathBuf::from(content.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_wbg() -> Option<PathBuf> {
        let output = Command::new("pgrep").arg("-a").arg("wbg").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let path = PathBuf::from(parts[1]);
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    fn from_waypaper() -> Option<PathBuf> {
        let conf = dirs::config_dir()?.join("waypaper/config.ini");
        if !conf.exists() {
            return None;
        }
        let content = std::fs::read_to_string(conf).ok()?;
        for line in content.lines() {
            if line.starts_with("wallpaper") {
                if let Some((_, path_str)) = line.split_once('=') {
                    let path = PathBuf::from(path_str.trim().trim_matches('"'));
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    fn fallback_most_recent() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        let dirs_to_check = vec![
            dirs::picture_dir(),
            dirs::picture_dir().map(|p| p.join("Wallpapers")),
            dirs::picture_dir().map(|p| p.join("Wallpaper")),
            dirs::home_dir().map(|p| p.join("Imágenes").join("Wallpapers-Axenide")), // añadido para el usuario
            dirs::home_dir().map(|p| p.join("Imágenes")),
            dirs::config_dir().map(|p| p.join("hypr")),
        ];
        for dir in dirs_to_check.into_iter().flatten() {
            if dir.exists() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(ext) = path.extension() {
                                let ext = ext.to_string_lossy().to_lowercase();
                                if matches!(ext.as_str(), "jpg"|"jpeg"|"png"|"bmp"|"webp"|"gif") {
                                    candidates.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }
        candidates.sort_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
        });
        candidates.into_iter().last()
    }

    // --- Carga de imagen (soporta GIF y video vía ffmpeg) ---
    fn load_image_from_path(path: &PathBuf) -> Result<image::DynamicImage> {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "avi" | "mov") {
            let temp_frame = std::env::temp_dir().join("cava_bg_temp_frame.png");
            let status = Command::new("ffmpeg")
                .args(["-i", path.to_str().unwrap(), "-vframes", "1", "-q:v", "2", temp_frame.to_str().unwrap(), "-y"])
                .status();
            if let Ok(status) = status {
                if status.success() {
                    let img = image::open(&temp_frame)
                        .context("Failed to open video frame")?;
                    let _ = std::fs::remove_file(temp_frame);
                    return Ok(img);
                }
            }
            anyhow::bail!("Could not extract frame from video");
        }

        image::open(path).context("Failed to open image")
    }

    // --- Paleta de colores por defecto (Catppuccin) ---
    pub fn default_colors(num_colors: usize) -> Vec<[f32; 4]> {
        let catppuccin = [
            [0.580, 0.886, 0.835, 1.0],
            [0.537, 0.863, 0.922, 1.0],
            [0.455, 0.780, 0.925, 1.0],
            [0.537, 0.706, 0.980, 1.0],
            [0.796, 0.651, 0.969, 1.0],
            [0.961, 0.761, 0.906, 1.0],
            [0.922, 0.627, 0.675, 1.0],
            [0.953, 0.545, 0.659, 1.0],
        ];
        if num_colors <= catppuccin.len() {
            catppuccin[0..num_colors].to_vec()
        } else {
            let mut colors = Vec::new();
            for i in 0..num_colors {
                colors.push(catppuccin[i % catppuccin.len()]);
            }
            colors
        }
    }

    /// Genera una paleta de colores a partir del archivo de wallpaper actual.
    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper() {
            Some(path) => path,
            None => {
                log::warn!("No wallpaper found, using default colors");
                return Ok(Self::default_colors(num_colors));
            }
        };

        log::info!("Analyzing wallpaper: {:?}", wallpaper_path);

        let img = match Self::load_image_from_path(&wallpaper_path) {
            Ok(img) => img,
            Err(e) => {
                log::warn!("Could not load wallpaper image: {}, using default colors", e);
                return Ok(Self::default_colors(num_colors));
            }
        };

        let (width, height) = img.dimensions();
        log::debug!("Wallpaper dimensions: {}x{}", width, height);
        let mut new_colors = Self::extract_and_generate_gradient(&img, num_colors);

        let mut prev_guard = PREVIOUS_COLORS.lock().unwrap();
        if !prev_guard.is_empty() && prev_guard.len() == new_colors.len() {
            for i in 0..new_colors.len() {
                for c in 0..4 {
                    new_colors[i][c] = COLOR_SMOOTHING_FACTOR * new_colors[i][c]
                        + (1.0 - COLOR_SMOOTHING_FACTOR) * prev_guard[i][c];
                }
            }
        }
        *prev_guard = new_colors.clone();
        Ok(new_colors)
    }

    fn extract_and_generate_gradient(img: &image::DynamicImage, num_colors: usize) -> Vec<[f32; 4]> {
        let (width, height) = img.dimensions();
        let mut samples = Vec::new();
        let step = (width * height / 10000).max(1) as u32;
        for y in (0..height).step_by(step as usize) {
            for x in (0..width).step_by(step as usize) {
                let pixel = img.get_pixel(x, y);
                let rgb = pixel.to_rgb();
                let channels = rgb.channels();
                let brightness = (channels[0] as f32 + channels[1] as f32 + channels[2] as f32) / 3.0;
                let max_ch = channels[0].max(channels[1]).max(channels[2]) as f32;
                let min_ch = channels[0].min(channels[1]).min(channels[2]) as f32;
                let saturation = if max_ch > 0.0 { (max_ch - min_ch) / max_ch } else { 0.0 };
                let is_bw = saturation < 0.1;
                if is_bw {
                    if brightness > 20.0 && brightness < 230.0 {
                        samples.push([channels[0] as f32, channels[1] as f32, channels[2] as f32]);
                    }
                } else {
                    if brightness > 50.0 && saturation > 0.2 {
                        samples.push([channels[0] as f32, channels[1] as f32, channels[2] as f32]);
                    }
                }
            }
        }
        if samples.is_empty() {
            for y in (0..height).step_by((step * 2) as usize) {
                for x in (0..width).step_by((step * 2) as usize) {
                    let pixel = img.get_pixel(x, y);
                    let rgb = pixel.to_rgb();
                    let channels = rgb.channels();
                    samples.push([channels[0] as f32, channels[1] as f32, channels[2] as f32]);
                }
            }
        }
        if samples.is_empty() {
            return Self::default_colors(num_colors);
        }
        let dominant = Self::find_dominant_colors(&samples, 4.min(samples.len()));
        Self::generate_gradient_palette(&dominant, num_colors)
    }

    fn find_dominant_colors(samples: &[[f32; 3]], k: usize) -> Vec<[f32; 3]> {
        if samples.len() <= k {
            return samples.to_vec();
        }
        let mut centroids: Vec<[f32; 3]> = (0..k).map(|i| samples[i * samples.len() / k]).collect();
        for _ in 0..5 {
            let mut clusters: Vec<Vec<[f32; 3]>> = vec![Vec::new(); centroids.len()];
            for sample in samples {
                let mut min_dist = f32::MAX;
                let mut cluster_idx = 0;
                for (i, cent) in centroids.iter().enumerate() {
                    let d = Self::color_distance(sample, cent);
                    if d < min_dist {
                        min_dist = d;
                        cluster_idx = i;
                    }
                }
                clusters[cluster_idx].push(*sample);
            }
            let mut new_centroids = Vec::new();
            for cluster in clusters {
                if cluster.is_empty() {
                    new_centroids.push(samples[new_centroids.len() % samples.len()]);
                } else {
                    let mut sum = [0.0, 0.0, 0.0];
                    for s in &cluster {
                        sum[0] += s[0];
                        sum[1] += s[1];
                        sum[2] += s[2];
                    }
                    let n = cluster.len() as f32;
                    new_centroids.push([sum[0] / n, sum[1] / n, sum[2] / n]);
                }
            }
            centroids = new_centroids;
        }
        centroids
    }

    fn generate_gradient_palette(dominant: &[[f32; 3]], num_colors: usize) -> Vec<[f32; 4]> {
        if dominant.is_empty() {
            return Self::default_colors(num_colors);
        }
        let is_bw = dominant.iter().all(|c| {
            let max = c[0].max(c[1]).max(c[2]);
            let min = c[0].min(c[1]).min(c[2]);
            let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
            sat < 0.1
        });
        if is_bw {
            let colorful = vec![
                [0.2, 0.4, 0.8, 1.0],
                [0.4, 0.6, 0.9, 1.0],
                [0.6, 0.4, 0.9, 1.0],
                [0.8, 0.2, 0.8, 1.0],
                [0.9, 0.4, 0.6, 1.0],
                [0.9, 0.6, 0.4, 1.0],
                [0.8, 0.8, 0.2, 1.0],
                [0.6, 0.9, 0.4, 1.0],
            ];
            return colorful.into_iter().take(num_colors).collect();
        }
        let mut palette = Vec::new();
        for i in 0..num_colors {
            let t = i as f32 / (num_colors - 1) as f32;
            let seg = t * (dominant.len() - 1) as f32;
            let idx = seg.floor() as usize;
            let frac = seg - idx as f32;
            if idx >= dominant.len() - 1 {
                let c = dominant.last().unwrap();
                palette.push([c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, 1.0]);
            } else {
                let c1 = dominant[idx];
                let c2 = dominant[idx + 1];
                palette.push([
                    (c1[0] + (c2[0] - c1[0]) * frac) / 255.0,
                    (c1[1] + (c2[1] - c1[1]) * frac) / 255.0,
                    (c1[2] + (c2[2] - c1[2]) * frac) / 255.0,
                    1.0,
                ]);
            }
        }
        for c in &mut palette {
            c[0] = c[0].clamp(0.0, 1.0);
            c[1] = c[1].clamp(0.0, 1.0);
            c[2] = c[2].clamp(0.0, 1.0);
        }
        palette
    }

    fn color_distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        (dr * dr * 0.299 + dg * dg * 0.587 + db * db * 0.114).sqrt()
    }
}