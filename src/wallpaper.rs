use anyhow::{Context, Result};
use gazo;
use image::RgbaImage;
use log;
use once_cell::sync::Lazy;
use rgb::ComponentBytes;
use std::sync::Mutex;

static PREVIOUS_COLORS: Lazy<Mutex<Vec<[f32; 4]>>> = Lazy::new(|| Mutex::new(Vec::new()));
const COLOR_SMOOTHING_FACTOR: f32 = 0.5;

pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    /// Captura la pantalla actual y extrae una paleta de colores.
    pub fn capture_and_extract_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        // Capturar todas las salidas (pantalla completa). false = no incluir cursor.
        let capture = gazo::capture_all_outputs(false)
            .context("Failed to capture screen with gazo")?;

        log::debug!("Captured image: {}x{}", capture.width, capture.height);

        // Convertir los datos de la captura a una imagen RgbaImage.
        // Los datos están en formato RGBA8 y se pueden obtener como un slice de bytes.
        let img = RgbaImage::from_raw(
            capture.width as u32,
            capture.height as u32,
            capture.pixel_data.as_bytes().to_vec(),
        ).context("Failed to create image from capture data")?;

        // Extraer colores
        let new_colors = Self::extract_and_generate_gradient(&img, num_colors);

        // Suavizar con colores anteriores
        let mut prev_guard = PREVIOUS_COLORS.lock().unwrap();
        let smoothed = if !prev_guard.is_empty() && prev_guard.len() == new_colors.len() {
            new_colors
                .iter()
                .enumerate()
                .map(|(i, &color)| {
                    let prev = prev_guard[i];
                    [
                        COLOR_SMOOTHING_FACTOR * color[0] + (1.0 - COLOR_SMOOTHING_FACTOR) * prev[0],
                        COLOR_SMOOTHING_FACTOR * color[1] + (1.0 - COLOR_SMOOTHING_FACTOR) * prev[1],
                        COLOR_SMOOTHING_FACTOR * color[2] + (1.0 - COLOR_SMOOTHING_FACTOR) * prev[2],
                        COLOR_SMOOTHING_FACTOR * color[3] + (1.0 - COLOR_SMOOTHING_FACTOR) * prev[3],
                    ]
                })
                .collect()
        } else {
            new_colors
        };

        *prev_guard = smoothed.clone();
        Ok(smoothed)
    }

    fn extract_and_generate_gradient(img: &RgbaImage, num_colors: usize) -> Vec<[f32; 4]> {
        let (width, height) = img.dimensions();
        let mut samples = Vec::new();

        // Muestrear la imagen para obtener píxeles relevantes
        let step = (width * height / 10000).max(1);
        for y in (0..height).step_by(step as usize) {
            for x in (0..width).step_by(step as usize) {
                let pixel = img.get_pixel(x, y);
                let channels = pixel.0;
                let r = channels[0] as f32;
                let g = channels[1] as f32;
                let b = channels[2] as f32;
                let brightness = (r + g + b) / 3.0;
                let max_ch = r.max(g).max(b);
                let min_ch = r.min(g).min(b);
                let saturation = if max_ch > 0.0 { (max_ch - min_ch) / max_ch } else { 0.0 };
                let is_bw = saturation < 0.1;

                if is_bw {
                    if brightness > 20.0 && brightness < 230.0 {
                        samples.push([r, g, b]);
                    }
                } else {
                    if brightness > 50.0 && saturation > 0.2 {
                        samples.push([r, g, b]);
                    }
                }
            }
        }

        // Si no se encontraron muestras suficientes, usar un muestreo más denso
        if samples.is_empty() {
            for y in (0..height).step_by((step * 2) as usize) {
                for x in (0..width).step_by((step * 2) as usize) {
                    let pixel = img.get_pixel(x, y);
                    let channels = pixel.0;
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

    fn color_distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        (dr * dr * 0.299 + dg * dg * 0.587 + db * db * 0.114).sqrt()
    }
}