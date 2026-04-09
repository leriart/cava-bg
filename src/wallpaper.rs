use anyhow::{Context, Result};
use image::{GenericImageView, Pixel};
use palette::{FromColor, Hsv, LinSrgb, Srgb};
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
                })
                .ok(),
            dirs::config_dir()
                .map(|mut p| {
                    p.push("hypr");
                    p.push("wallpaper.png");
                    p
                })
                .ok(),
            // Sway
            dirs::config_dir()
                .map(|mut p| {
                    p.push("sway");
                    p.push("wallpaper");
                    p
                })
                .ok(),
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

    /// Extract dominant colors from an image
    pub fn extract_colors(image_path: &PathBuf, num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let img = image::open(image_path)
            .with_context(|| format!("Failed to open image: {}", image_path.display()))?;

        let (width, height) = img.dimensions();
        
        // Sample pixels from the image (every 10th pixel for performance)
        let mut samples = Vec::new();
        for y in (0..height).step_by(10) {
            for x in (0..width).step_by(10) {
                let pixel = img.get_pixel(x, y);
                let rgb = pixel.to_rgb();
                samples.push(LinSrgb::new(
                    rgb[0] as f32 / 255.0,
                    rgb[1] as f32 / 255.0,
                    rgb[2] as f32 / 255.0,
                ));
            }
        }

        if samples.is_empty() {
            return Ok(Self::default_colors(num_colors));
        }

        // Convert to HSV for better color clustering
        let hsv_samples: Vec<Hsv> = samples.iter().map(|&rgb| Hsv::from_color(rgb)).collect();

        // Simple k-means clustering (simplified)
        let mut colors = Vec::new();
        
        // Group by hue ranges
        let hue_ranges = if num_colors <= 3 {
            vec![(0.0, 360.0)]
        } else {
            // Divide hue circle into segments
            let segment_size = 360.0 / num_colors as f32;
            (0..num_colors)
                .map(|i| (i as f32 * segment_size, (i + 1) as f32 * segment_size))
                .collect()
        };

        for (hue_start, hue_end) in hue_ranges {
            let mut total_r = 0.0;
            let mut total_g = 0.0;
            let mut total_b = 0.0;
            let mut count = 0;

            for (hsv, &rgb) in hsv_samples.iter().zip(samples.iter()) {
                let hue = if hsv.hue.to_positive_degrees() < hue_start {
                    hsv.hue.to_positive_degrees() + 360.0
                } else {
                    hsv.hue.to_positive_degrees()
                };

                if hue >= hue_start && hue <= hue_end {
                    total_r += rgb.red;
                    total_g += rgb.green;
                    total_b += rgb.blue;
                    count += 1;
                }
            }

            if count > 0 {
                let avg_r = total_r / count as f32;
                let avg_g = total_g / count as f32;
                let avg_b = total_b / count as f32;
                
                // Convert back to sRGB and ensure values are in [0, 1]
                let srgb = Srgb::new(avg_r, avg_g, avg_b);
                colors.push([
                    srgb.red.max(0.0).min(1.0),
                    srgb.green.max(0.0).min(1.0),
                    srgb.blue.max(0.0).min(1.0),
                    1.0, // Alpha
                ]);
            }
        }

        // If we didn't get enough colors, fill with defaults
        while colors.len() < num_colors {
            colors.extend(Self::default_colors(num_colors - colors.len()));
        }

        // Limit to requested number of colors
        colors.truncate(num_colors);

        Ok(colors)
    }

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
    pub fn generate_gradient_colors(num_colors: usize) -> Result<Vec<[f32; 4]>> {
        if let Some(wallpaper_path) = Self::find_wallpaper()? {
            match Self::extract_colors(&wallpaper_path, num_colors) {
                Ok(colors) => {
                    log::info!(
                        "Extracted {} colors from wallpaper: {}",
                        colors.len(),
                        wallpaper_path.display()
                    );
                    return Ok(colors);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to extract colors from wallpaper ({}): {}",
                        wallpaper_path.display(),
                        e
                    );
                }
            }
        }

        log::info!("Using default color palette");
        Ok(Self::default_colors(num_colors))
    }
}