use log::{info, warn};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct DiscoveredParallaxLayer {
    #[allow(dead_code)]
    pub path: PathBuf,
    pub inferred_depth: f32,
}

#[allow(dead_code)]
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "webp", "gif", "apng", "tiff", "mp4", "webm", "mkv", "avi", "mov",
    "m4v", "flv", "wmv",
];

#[allow(dead_code)]
pub fn extract_basename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().trim().to_string();
    if stem.is_empty() {
        return None;
    }
    Some(stem)
}

/// Match X-Ray layers based on wallpaper filename.
///
/// If wallpaper is: /path/to/sunset.mp4
/// looks for:
/// - sunset_base.*
/// - sunset_reveal.*
#[allow(dead_code)]
pub fn find_matching_layers(wallpaper_path: &str, xray_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let wallpaper_name = Path::new(wallpaper_path).file_stem()?.to_str()?.trim();
    if wallpaper_name.is_empty() {
        return None;
    }

    info!("Looking for X-Ray layers matching: {}", wallpaper_name);

    let wallpaper_name_lc = wallpaper_name.to_lowercase();
    let base_pattern = format!("{}_base", wallpaper_name);
    let reveal_pattern = format!("{}_reveal", wallpaper_name);

    let base_pattern_lc = base_pattern.to_lowercase();
    let reveal_pattern_lc = reveal_pattern.to_lowercase();

    let mut best_base: Option<(u8, PathBuf)> = None;
    let mut best_reveal: Option<(u8, PathBuf)> = None;
    let mut exact_wallpaper_match: Option<PathBuf> = None;

    for entry in WalkDir::new(xray_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        if !is_supported_layer_file(&path) {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let stem_lc = stem.to_lowercase();

        if stem_lc == wallpaper_name_lc && exact_wallpaper_match.is_none() {
            exact_wallpaper_match = Some(path.clone());
            info!(
                "Found exact wallpaper-name layer candidate (base fallback): {:?}",
                path
            );
        }

        if let Some(score) = candidate_score(&stem_lc, &base_pattern_lc, "layer1") {
            let should_replace = best_base
                .as_ref()
                .map(|(best_score, _)| score < *best_score)
                .unwrap_or(true);
            if should_replace {
                best_base = Some((score, path.clone()));
                info!("Found base layer candidate (score {}): {:?}", score, path);
            }
        }

        if let Some(score) = candidate_score(&stem_lc, &reveal_pattern_lc, "layer2") {
            let should_replace = best_reveal
                .as_ref()
                .map(|(best_score, _)| score < *best_score)
                .unwrap_or(true);
            if should_replace {
                best_reveal = Some((score, path.clone()));
                info!("Found reveal layer candidate (score {}): {:?}", score, path);
            }
        }
    }

    let base_layer = best_base
        .map(|(_, path)| path)
        .or(exact_wallpaper_match.clone());
    let reveal_layer = best_reveal.map(|(_, path)| path);

    match (base_layer, reveal_layer) {
        (Some(base), Some(reveal)) => {
            info!(
                "Successfully matched X-Ray pair: base={:?}, reveal={:?}",
                base, reveal
            );
            Some((base, reveal))
        }
        _ => {
            warn!(
                "Could not find complete X-Ray pair for '{}'. Expected: {}.* and {}.* in {}",
                wallpaper_name,
                base_pattern,
                reveal_pattern,
                xray_dir.display()
            );
            None
        }
    }
}

#[allow(dead_code)]
fn candidate_score(stem_lc: &str, pattern_lc: &str, legacy_suffix: &str) -> Option<u8> {
    if stem_lc == pattern_lc {
        return Some(0);
    }

    if stem_lc.starts_with(&format!("{}_", pattern_lc)) {
        return Some(1);
    }

    let prefix = pattern_lc
        .trim_end_matches("_base")
        .trim_end_matches("_reveal");
    if stem_lc == format!("{}_{}", prefix, legacy_suffix) {
        return Some(2);
    }

    None
}

#[allow(dead_code)]
fn is_supported_layer_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    SUPPORTED_EXTENSIONS.contains(&ext.as_str())
}

#[allow(dead_code)]
pub fn discover_parallax_layers(parallax_dir: &Path) -> Vec<DiscoveredParallaxLayer> {
    let mut out = Vec::new();

    for entry in WalkDir::new(parallax_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        if !is_supported_layer_file(&path) {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        let inferred_depth = infer_depth_from_stem(&stem);
        out.push(DiscoveredParallaxLayer {
            path,
            inferred_depth,
        });
    }

    out.sort_by(|a, b| {
        a.inferred_depth
            .partial_cmp(&b.inferred_depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn infer_depth_from_stem(stem: &str) -> f32 {
    // Accept patterns such as "city_depth_0.3", "layer3", "bg_80".
    if let Some(depth) = stem.split('_').find_map(|token| token.parse::<f32>().ok()) {
        return depth.clamp(0.0, 1.0);
    }

    if let Ok(index) = stem
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .parse::<u32>()
    {
        return (index as f32 / 10.0).clamp(0.0, 1.0);
    }

    0.5
}
