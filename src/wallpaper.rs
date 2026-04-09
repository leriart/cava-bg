use anyhow::{Context, Result};
use image::{GenericImageView, Pixel};
use palette::{FromColor, Hsv, Lab, LinSrgb, Srgb};
use std::collections::HashMap;
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

    /// Extract dominant colors from an image using improved algorithm
    pub fn extract_colors(image_path: &PathBuf, num_colors: usize) -> Result<Vec<[f32; 4]>> {
        let img = image::open(image_path)
            .with_context(|| format!("Failed to open image: {}", image_path.display()))?;

        let (width, height) = img.dimensions();

        // Sample pixels from the image (every 8th pixel for performance)
        let mut color_counts: HashMap<[u8; 3], usize> = HashMap::new();
        let total_samples = (width as usize / 8) * (height as usize / 8);

        for y in (0..height).step_by(8) {
            for x in (0..width).step_by(8) {
                let pixel = img.get_pixel(x, y);
                let rgb = pixel.to_rgb();
                let quantized = [
                    (rgb[0] >> 4) << 4, // Quantize to 16 levels
                    (rgb[1] >> 4) << 4,
                    (rgb[2] >> 4) << 4,
                ];
                *color_counts.entry(quantized).or_insert(0) += 1;
            }
        }

        if color_counts.is_empty() {
            return Ok(Self::default_colors(num_colors));
        }

        // Convert to Lab color space for better clustering
        let mut lab_colors: Vec<(Lab, usize)> = color_counts
            .iter()
            .map(|(&rgb, &count)| {
                let srgb = Srgb::new(
                    rgb[0] as f32 / 255.0,
                    rgb[1] as f32 / 255.0,
                    rgb[2] as f32 / 255.0,
                );
                let lab: Lab = Lab::from_color(srgb.into_linear());
                (lab, count)
            })
            .collect();

        // Sort by frequency (most common colors first)
        lab_colors.sort_by(|a, b| b.1.cmp(&a.1));

        // Apply k-means clustering in Lab space
        let mut clusters = Vec::new();
        let mut used_indices = Vec::new();

        for i in 0..lab_colors.len() {
            if used_indices.contains(&i) {
                continue;
            }

            let (center_color, center_count) = &lab_colors[i];
            let mut total_lab = *center_color;
            let mut total_count = *center_count;
            let mut cluster_members = vec![i];

            // Find similar colors within threshold
            for j in (i + 1)..lab_colors.len() {
                if used_indices.contains(&j) {
                    continue;
                }

                let (other_color, other_count) = &lab_colors[j];
                let distance = Self::color_distance_lab(*center_color, *other_color);

                // Threshold for similar colors (adjustable)
                if distance < 15.0 {
                    total_lab = Lab::new(
                        total_lab.l + other_color.l * (*other_count as f32),
                        total_lab.a + other_color.a * (*other_count as f32),
                        total_lab.b + other_color.b * (*other_count as f32),
                    );
                    total_count += other_count;
                    cluster_members.push(j);
                }
            }

            // Calculate weighted average for cluster
            let avg_lab = Lab::new(
                total_lab.l / total_count as f32,
                total_lab.a / total_count as f32,
                total_lab.b / total_count as f32,
            );

            // Convert back to sRGB
            let lin_srgb: LinSrgb = LinSrgb::from_color(avg_lab);
            let srgb = Srgb::from_linear(lin_srgb);

            clusters.push((
                [
                    srgb.red.max(0.0).min(1.0),
                    srgb.green.max(0.0).min(1.0),
                    srgb.blue.max(0.0).min(1.0),
                    1.0,
                ],
                total_count,
            ));

            // Mark all members as used
            used_indices.extend(cluster_members);
        }

        // Sort clusters by total pixel count (dominance)
        clusters.sort_by(|a, b| b.1.cmp(&a.1));

        // Select top N colors
        let mut selected_colors: Vec<[f32; 4]> = clusters
            .iter()
            .take(num_colors)
            .map(|(color, _)| *color)
            .collect();

        // If we don't have enough colors, generate complementary ones
        if selected_colors.len() < num_colors {
            selected_colors = Self::generate_harmonious_colors(&selected_colors, num_colors);
        }

        // Ensure colors have good contrast and vibrancy
        selected_colors = Self::enhance_colors(&selected_colors);

        Ok(selected_colors)
    }

    /// Calculate color distance in Lab space (perceptually uniform)
    fn color_distance_lab(lab1: Lab, lab2: Lab) -> f32 {
        let dl = lab1.l - lab2.l;
        let da = lab1.a - lab2.a;
        let db = lab1.b - lab2.b;
        (dl * dl + da * da + db * db).sqrt()
    }

    /// Generate harmonious colors based on extracted ones
    fn generate_harmonious_colors(base_colors: &[[f32; 4]], target_count: usize) -> Vec<[f32; 4]> {
        let mut colors = base_colors.to_vec();

        if colors.is_empty() {
            return Self::default_colors(target_count);
        }

        // Convert to HSV for color manipulation
        let mut hsv_colors: Vec<Hsv> = colors
            .iter()
            .map(|color| {
                let srgb = Srgb::new(color[0], color[1], color[2]);
                Hsv::from_color(srgb.into_linear())
            })
            .collect();

        // Generate complementary, analogous, and triadic colors
        while colors.len() < target_count {
            let last_color = &hsv_colors[colors.len() % hsv_colors.len()];

            // Alternate between different color relationships
            match colors.len() % 3 {
                0 => {
                    // Complementary color (180° hue shift)
                    let mut new_hsv = *last_color;
                    new_hsv.hue = new_hsv.hue + 180.0;
                    let new_srgb: Srgb = Srgb::from_color(LinSrgb::from_color(new_hsv));
                    colors.push([
                        new_srgb.red.max(0.0).min(1.0),
                        new_srgb.green.max(0.0).min(1.0),
                        new_srgb.blue.max(0.0).min(1.0),
                        1.0,
                    ]);
                    hsv_colors.push(new_hsv);
                }
                1 => {
                    // Analogous color (30° hue shift)
                    let mut new_hsv = *last_color;
                    new_hsv.hue = new_hsv.hue + 30.0;
                    let new_srgb: Srgb = Srgb::from_color(LinSrgb::from_color(new_hsv));
                    colors.push([
                        new_srgb.red.max(0.0).min(1.0),
                        new_srgb.green.max(0.0).min(1.0),
                        new_srgb.blue.max(0.0).min(1.0),
                        1.0,
                    ]);
                    hsv_colors.push(new_hsv);
                }
                _ => {
                    // Triadic color (120° hue shift)
                    let mut new_hsv = *last_color;
                    new_hsv.hue = new_hsv.hue + 120.0;
                    let new_srgb: Srgb = Srgb::from_color(LinSrgb::from_color(new_hsv));
                    colors.push([
                        new_srgb.red.max(0.0).min(1.0),
                        new_srgb.green.max(0.0).min(1.0),
                        new_srgb.blue.max(0.0).min(1.0),
                        1.0,
                    ]);
                    hsv_colors.push(new_hsv);
                }
            }
        }

        colors.truncate(target_count);
        colors
    }

    /// Enhance colors for better visibility and contrast
    fn enhance_colors(colors: &[[f32; 4]]) -> Vec<[f32; 4]> {
        colors
            .iter()
            .map(|color| {
                let mut hsv: Hsv = Hsv::from_color(LinSrgb::new(color[0], color[1], color[2]));

                // Increase saturation for more vibrant colors
                hsv.saturation = hsv.saturation.min(0.9).max(0.7);

                // Ensure good brightness
                hsv.value = hsv.value.max(0.6).min(0.9);

                let srgb: Srgb = Srgb::from_color(LinSrgb::from_color(hsv));
                [
                    srgb.red.max(0.0).min(1.0),
                    srgb.green.max(0.0).min(1.0),
                    srgb.blue.max(0.0).min(1.0),
                    1.0,
                ]
            })
            .collect()
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
