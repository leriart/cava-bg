use anyhow::{Context, Result};
use image::{GenericImageView, Pixel};
use log;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Detect dominant colors from the current wallpaper
pub struct WallpaperAnalyzer;

impl WallpaperAnalyzer {
    /// Find the current wallpaper path
    pub fn find_wallpaper() -> Result<Option<PathBuf>> {
        // Common wallpaper locations
        let possible_paths = [
            // Hyprland
            dirs::config_dir()
                .map(|mut p| {
                    p.push("hypr");
                    p.push("wallpaper.jpg");
                    p
                }),
            dirs::config_dir()
                .map(|mut p| {
                    p.push("hypr");
                    p.push("wallpaper.png");
                    p
                }),
            // Sway
            dirs::config_dir()
                .map(|mut p| {
                    p.push("sway");
                    p.push("wallpaper");
                    p
                }),
            // Common locations
            dirs::picture_dir().map(|mut p| {
                p.push("wallpaper");
                p
            }),
            dirs::picture_dir().map(|mut p| {
                p.push("wallpaper.jpg");
                p
            }),
            dirs::picture_dir().map(|mut p| {
                p.push("wallpaper.png");
                p
            }),
            // Check common image files in home directory
            dirs::home_dir().map(|mut p| {
                p.push(".wallpaper");
                p
            }),
        ];

        for path in possible_paths.iter().flatten() {
            if path.exists() {
                return Ok(Some(path.clone()));
            }
        }

        // Search for image files in common directories
        let search_dirs = [
            dirs::picture_dir(),
            dirs::config_dir().map(|mut p| {
                p.push("hypr");
                p
            }),
            dirs::home_dir(),
        ];

        for dir in search_dirs.iter().flatten() {
            for entry in WalkDir::new(dir)
                .max_depth(2)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy().to_lowercase();
                        if matches!(
                            ext_str.as_str(),
                            "jpg" | "jpeg" | "png" | "bmp" | "webp" | "gif"
                        ) {
                            // Check for common wallpaper names
                            let filename = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_lowercase();
                            if filename.contains("wallpaper")
                                || filename.contains("background")
                                || filename.contains("bg")
                            {
                                return Ok(Some(path.to_path_buf()));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Extract dominant colors from an image using simple algorithm


    /// Get default colors (fallback when no wallpaper is found)
    pub fn default_colors(num_colors: usize) -> Vec<[f32; 4]> {
        // Catppuccin Mocha color palette
        let catppuccin = [
            [0.580, 0.886, 0.835, 1.0], // #94e2d5 - Teal
            [0.537, 0.863, 0.922, 1.0], // #89dceb - Sky
            [0.455, 0.780, 0.925, 1.0], // #74c7ec - Sapphire
            [0.537, 0.706, 0.980, 1.0], // #89b4fa - Blue
            [0.796, 0.651, 0.969, 1.0], // #cba6f7 - Lavender
            [0.961, 0.761, 0.906, 1.0], // #f5c2e7 - Pink
            [0.922, 0.627, 0.675, 1.0], // #eba0ac - Maroon
            [0.953, 0.545, 0.659, 1.0], // #f38ba8 - Red
        ];

        if num_colors <= catppuccin.len() {
            catppuccin[0..num_colors].to_vec()
        } else {
            // Repeat the palette if more colors are needed
            let mut colors = Vec::new();
            for i in 0..num_colors {
                colors.push(catppuccin[i % catppuccin.len()]);
            }
            colors
        }
    }

    /// Generate gradient colors based on wallpaper or defaults
    /// Generate gradient colors from wallpaper
    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let wallpaper_path = match Self::find_wallpaper()? {
            Some(path) => path,
            None => {
                // No wallpaper found, return default colors
                log::warn!("No wallpaper found, using default colors");
                return Ok(Self::default_colors(num_colors));
            }
        };

        log::info!("Analyzing wallpaper: {:?}", wallpaper_path);

        // Load image
        let img = image::open(&wallpaper_path)
            .context(format!("Failed to open wallpaper: {:?}", wallpaper_path))?;

        let (width, height) = img.dimensions();
        log::info!("Wallpaper dimensions: {}x{}", width, height);

        // Extract dominant colors and generate gradient palette
        let colors = Self::extract_and_generate_gradient(&img, num_colors);

        Ok(colors)
    }

    /// Get current wallpaper path (for change detection)
    pub fn get_current_wallpaper_path() -> Result<Option<PathBuf>> {
        Self::find_wallpaper()
    }

    /// Extract dominant colors and generate gradient palette
    fn extract_and_generate_gradient(img: &image::DynamicImage, num_colors: usize) -> Vec<[f32; 4]> {
        let (width, height) = img.dimensions();
        
        // Sample pixels from different regions
        let mut samples = Vec::new();
        let step = (width * height / 10000).max(1) as u32; // Sample ~10000 pixels
        
        for y in (0..height).step_by(step as usize) {
            for x in (0..width).step_by(step as usize) {
                let pixel = img.get_pixel(x, y);
                let rgb = pixel.to_rgb();
                let channels = rgb.channels();
                // Only include sufficiently bright and saturated colors
                let brightness = (channels[0] as f32 + channels[1] as f32 + channels[2] as f32) / 3.0;
                let max_channel = channels[0].max(channels[1]).max(channels[2]) as f32;
                let min_channel = channels[0].min(channels[1]).min(channels[2]) as f32;
                let saturation = if max_channel > 0.0 { (max_channel - min_channel) / max_channel } else { 0.0 };
                
                // For black and white wallpapers, we need different thresholds
                let is_black_white = saturation < 0.1;
                
                if is_black_white {
                    // For black/white images, accept based on brightness only
                    if brightness > 20.0 && brightness < 230.0 { // Avoid pure black and pure white
                        samples.push([channels[0] as f32, channels[1] as f32, channels[2] as f32]);
                    }
                } else {
                    // For colored images, use stricter thresholds
                    if brightness > 50.0 && saturation > 0.2 {
                        samples.push([channels[0] as f32, channels[1] as f32, channels[2] as f32]);
                    }
                }
            }
        }

        if samples.is_empty() {
            // Fall back to sampling all pixels
            for y in (0..height).step_by(step as usize * 2) {
                for x in (0..width).step_by(step as usize * 2) {
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

        // Find 3-4 dominant colors using k-means-like clustering
        let dominant_colors = Self::find_dominant_colors(&samples, 4.min(samples.len()));
        
        // Generate gradient colors by interpolating between dominant colors
        let gradient_colors = Self::generate_gradient_palette(&dominant_colors, num_colors);
        
        gradient_colors
    }

    /// Find dominant colors using simple clustering
    fn find_dominant_colors(samples: &[[f32; 3]], k: usize) -> Vec<[f32; 3]> {
        if samples.len() <= k {
            return samples.to_vec();
        }

        // Initialize with evenly spaced samples
        let mut centroids: Vec<[f32; 3]> = Vec::new();
        for i in 0..k {
            centroids.push(samples[i * samples.len() / k]);
        }

        // Simple clustering iterations
        for _ in 0..5 {
            let mut clusters: Vec<Vec<[f32; 3]>> = vec![Vec::new(); centroids.len()];
            
            // Assign samples to nearest centroid
            for sample in samples {
                let mut min_dist = f32::MAX;
                let mut cluster_idx = 0;
                
                for (i, centroid) in centroids.iter().enumerate() {
                    let dist = Self::color_distance(sample, centroid);
                    if dist < min_dist {
                        min_dist = dist;
                        cluster_idx = i;
                    }
                }
                
                clusters[cluster_idx].push(*sample);
            }
            
            // Update centroids
            let mut new_centroids = Vec::new();
            for cluster in &clusters {
                if cluster.is_empty() {
                    // Keep random sample if cluster is empty
                    new_centroids.push(samples[new_centroids.len() % samples.len()]);
                } else {
                    let mut sum = [0.0, 0.0, 0.0];
                    for sample in cluster {
                        sum[0] += sample[0];
                        sum[1] += sample[1];
                        sum[2] += sample[2];
                    }
                    let count = cluster.len() as f32;
                    new_centroids.push([
                        sum[0] / count,
                        sum[1] / count,
                        sum[2] / count,
                    ]);
                }
            }
            
            centroids = new_centroids;
        }
        
        centroids
    }

    /// Generate gradient palette by interpolating between colors
    fn generate_gradient_palette(dominant_colors: &[[f32; 3]], num_colors: usize) -> Vec<[f32; 4]> {
        if dominant_colors.is_empty() {
            return Self::default_colors(num_colors);
        }
        
        // Check if we have a black/white dominant palette
        let is_black_white_palette = dominant_colors.iter().all(|color| {
            let r = color[0];
            let g = color[1];
            let b = color[2];
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
            saturation < 0.1
        });
        
        let mut palette = Vec::new();
        
        if is_black_white_palette {
            // For black/white wallpapers, create a colorful gradient
            // Use a nice blue-to-purple gradient as default for B/W wallpapers
            let colorful_gradient = vec![
                [0.2, 0.4, 0.8, 1.0],   // Blue
                [0.4, 0.6, 0.9, 1.0],   // Light Blue
                [0.6, 0.4, 0.9, 1.0],   // Purple
                [0.8, 0.2, 0.8, 1.0],   // Magenta
                [0.9, 0.4, 0.6, 1.0],   // Pink
                [0.9, 0.6, 0.4, 1.0],   // Orange
                [0.8, 0.8, 0.2, 1.0],   // Yellow
                [0.6, 0.9, 0.4, 1.0],   // Green
            ];
            
            // Take the first num_colors from our colorful gradient
            for i in 0..num_colors.min(colorful_gradient.len()) {
                palette.push(colorful_gradient[i]);
            }
            
            // Fill remaining with defaults if needed
            while palette.len() < num_colors {
                palette.extend(Self::default_colors(num_colors - palette.len()));
            }
        } else if dominant_colors.len() == 1 {
            // Single color - create variations
            let base = dominant_colors[0];
            for i in 0..num_colors {
                let t = i as f32 / (num_colors - 1) as f32;
                // Vary brightness
                let brightness = 0.5 + t * 0.5;
                let color = [
                    (base[0] / 255.0) * brightness,
                    (base[1] / 255.0) * brightness,
                    (base[2] / 255.0) * brightness,
                    1.0,
                ];
                palette.push(color);
            }
        } else {
            // Multiple colors - create gradient between them
            for i in 0..num_colors {
                let t = i as f32 / (num_colors - 1) as f32;
                let segment = t * (dominant_colors.len() - 1) as f32;
                let idx = segment.floor() as usize;
                let frac = segment - idx as f32;
                
                if idx >= dominant_colors.len() - 1 {
                    // Last segment
                    let color = dominant_colors[dominant_colors.len() - 1];
                    palette.push([color[0] / 255.0, color[1] / 255.0, color[2] / 255.0, 1.0]);
                } else {
                    // Interpolate between two colors
                    let color1 = dominant_colors[idx];
                    let color2 = dominant_colors[idx + 1];
                    let color = [
                        (color1[0] + (color2[0] - color1[0]) * frac) / 255.0,
                        (color1[1] + (color2[1] - color1[1]) * frac) / 255.0,
                        (color1[2] + (color2[2] - color1[2]) * frac) / 255.0,
                        1.0,
                    ];
                    palette.push(color);
                }
            }
        }
        
        // Ensure colors are in valid range
        for color in &mut palette {
            color[0] = color[0].clamp(0.0, 1.0);
            color[1] = color[1].clamp(0.0, 1.0);
            color[2] = color[2].clamp(0.0, 1.0);
        }
        
        palette
    }

    /// Calculate color distance (perceptually weighted)
    fn color_distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        // Weighted for human perception
        (dr * dr * 0.299 + dg * dg * 0.587 + db * db * 0.114).sqrt()
    }


}