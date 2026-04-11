use anyhow::{Context, Result};
use image::{GenericImageView, Pixel};
use log;
use std::path::PathBuf;
use walkdir::WalkDir;
use std::sync::Mutex;
use once_cell::sync::Lazy;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.5;

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    pub fn find_wallpaper() -> Result<Option<PathBuf>> {
        let possible_paths = [
            dirs::config_dir().map(|mut p| { p.push("hypr"); p.push("wallpaper.jpg"); p }),
            dirs::config_dir().map(|mut p| { p.push("hypr"); p.push("wallpaper.png"); p }),
            dirs::config_dir().map(|mut p| { p.push("sway"); p.push("wallpaper"); p }),
            dirs::picture_dir().map(|mut p| { p.push("wallpaper"); p }),
            dirs::picture_dir().map(|mut p| { p.push("wallpaper.jpg"); p }),
            dirs::picture_dir().map(|mut p| { p.push("wallpaper.png"); p }),
            dirs::home_dir().map(|mut p| { p.push(".wallpaper"); p }),
        ];
        for path in possible_paths.iter().flatten() {
            if path.exists() {
                return Ok(Some(path.clone()));
            }
        }

        let search_dirs = [
            dirs::picture_dir(),
            dirs::config_dir().map(|mut p| { p.push("hypr"); p }),
            dirs::home_dir(),
        ];
        for dir in search_dirs.iter().flatten() {
            for entry in WalkDir::new(dir).max_depth(2).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy().to_lowercase();
                        if matches!(ext_str.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "webp" | "gif") {
                            let filename = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                            if filename.contains("wallpaper") || filename.contains("background") || filename.contains("bg") {
                                return Ok(Some(path.to_path_buf()));
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

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

    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper()? {
            Some(path) => path,
            None => {
                log::warn!("No wallpaper found, using default colors");
                return Ok(Self::default_colors(num_colors));
            }
        };

        // Si no es una imagen estática soportada, usar colores por defecto
        if let Some(ext) = wallpaper_path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "tiff" | "webp") {
                log::info!("Wallpaper is not a static image ({}), using default colors", ext);
                return Ok(Self::default_colors(num_colors));
            }
        }

        log::info!("Analyzing wallpaper: {:?}", wallpaper_path);
        let img = image::open(&wallpaper_path)
            .context(format!("Failed to open wallpaper: {:?}", wallpaper_path))?;
        let (width, height) = img.dimensions();
        log::info!("Wallpaper dimensions: {}x{}", width, height);
        let mut new_colors = Self::extract_and_generate_gradient(&img, num_colors);

        let mut prev_guard = PREVIOUS_COLORS.lock().unwrap();
        if !prev_guard.is_empty() && prev_guard.len() == new_colors.len() {
            for i in 0..new_colors.len() {
                for c in 0..4 {
                    new_colors[i][c] = COLOR_SMOOTHING_FACTOR * new_colors[i][c] + (1.0 - COLOR_SMOOTHING_FACTOR) * prev_guard[i][c];
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
            for y in (0..height).step_by((step*2) as usize) {
                for x in (0..width).step_by((step*2) as usize) {
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
                    if d < min_dist { min_dist = d; cluster_idx = i; }
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
                        sum[0] += s[0]; sum[1] += s[1]; sum[2] += s[2];
                    }
                    let n = cluster.len() as f32;
                    new_centroids.push([sum[0]/n, sum[1]/n, sum[2]/n]);
                }
            }
            centroids = new_centroids;
        }
        centroids
    }

    fn generate_gradient_palette(dominant: &[[f32; 3]], num_colors: usize) -> Vec<[f32; 4]> {
        if dominant.is_empty() { return Self::default_colors(num_colors); }
        let is_bw = dominant.iter().all(|c| {
            let max = c[0].max(c[1]).max(c[2]);
            let min = c[0].min(c[1]).min(c[2]);
            let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
            sat < 0.1
        });
        if is_bw {
            let colorful = vec![
                [0.2,0.4,0.8,1.0], [0.4,0.6,0.9,1.0], [0.6,0.4,0.9,1.0],
                [0.8,0.2,0.8,1.0], [0.9,0.4,0.6,1.0], [0.9,0.6,0.4,1.0],
                [0.8,0.8,0.2,1.0], [0.6,0.9,0.4,1.0]
            ];
            return colorful.into_iter().take(num_colors).map(|c| c).collect();
        }
        let mut palette = Vec::new();
        for i in 0..num_colors {
            let t = i as f32 / (num_colors-1) as f32;
            let seg = t * (dominant.len()-1) as f32;
            let idx = seg.floor() as usize;
            let frac = seg - idx as f32;
            if idx >= dominant.len()-1 {
                let c = dominant.last().unwrap();
                palette.push([c[0]/255.0, c[1]/255.0, c[2]/255.0, 1.0]);
            } else {
                let c1 = dominant[idx];
                let c2 = dominant[idx+1];
                palette.push([
                    (c1[0] + (c2[0]-c1[0])*frac)/255.0,
                    (c1[1] + (c2[1]-c1[1])*frac)/255.0,
                    (c1[2] + (c2[2]-c1[2])*frac)/255.0,
                    1.0
                ]);
            }
        }
        for c in &mut palette {
            c[0] = c[0].clamp(0.0,1.0);
            c[1] = c[1].clamp(0.0,1.0);
            c[2] = c[2].clamp(0.0,1.0);
        }
        palette
    }

    fn color_distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        let dr = a[0]-b[0]; let dg = a[1]-b[1]; let db = a[2]-b[2];
        (dr*dr*0.299 + dg*dg*0.587 + db*db*0.114).sqrt()
    }
}