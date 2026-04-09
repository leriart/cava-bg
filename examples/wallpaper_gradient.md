# Wallpaper Gradient Color System

cava-bg features an advanced gradient color generation system that automatically creates beautiful color palettes from your wallpaper.

## How It Works

### 1. Wallpaper Detection
The system searches for your wallpaper in common locations:
- `~/.config/hypr/wallpaper.{jpg,png}`
- `~/.config/sway/wallpaper`
- `~/Pictures/wallpaper.{jpg,png}`
- Common wallpaper directories

### 2. Color Extraction
Using an intelligent algorithm:
1. **Sampling**: Analyzes ~10,000 pixels from your wallpaper
2. **Filtering**: Focuses on bright, saturated colors for better visibility
3. **Clustering**: Groups similar colors to find dominant color families
4. **Selection**: Identifies 3-4 primary colors from your wallpaper

### 3. Gradient Generation
Creates smooth gradients between extracted colors:
- **Single Color Wallpapers**: Generates brightness variations
- **Multi-color Wallpapers**: Creates smooth transitions between colors
- **8-Color Palette**: Outputs 8 gradient colors for cava visualization

### 4. Automatic Updates
- **Change Detection**: Monitors wallpaper file for changes every 5 seconds
- **Real-time Updates**: Restarts cava with new colors when wallpaper changes
- **Fallback System**: Uses beautiful default colors if analysis fails

## Example Output

For a wallpaper with blue tones, you might get:
```
Color 1: #1e3a8a (Dark Blue)
Color 2: #3b82f6 (Blue)
Color 3: #60a5fa (Light Blue)
Color 4: #93c5fd (Very Light Blue)
Color 5: #bfdbfe (Pale Blue)
Color 6: #dbeafe (Almost White Blue)
Color 7: #eff6ff (White with Blue Tint)
Color 8: #f8fafc (Near White)
```

## Configuration Options

In `~/.config/cava-bg/config.toml`:

```toml
[general]
# Enable/disable automatic wallpaper change detection
auto_detect_wallpaper_changes = true

# How often to check for wallpaper changes (seconds)
wallpaper_check_interval = 5
```

## Manual Testing

Test the gradient generation system:
```bash
cava-bg --test-config
```

This will:
1. Load your configuration
2. Analyze your current wallpaper
3. Display the extracted gradient colors
4. Show color values in RGB and hexadecimal formats

## Fallback Colors

If wallpaper analysis fails, cava-bg uses a beautiful Catppuccin Mocha gradient:
- `#94e2d5` (Mint)
- `#89dceb` (Sky Blue)
- `#74c7ec` (Blue)
- `#89b4fa` (Lavender)
- `#cba6f7` (Mauve)
- `#f5c2e7` (Pink)
- `#eba0ac` (Maroon)
- `#f38ba8` (Red)

## Technical Details

The algorithm uses:
- **Perceptual Color Distance**: Weighted for human vision (YUV-like)
- **Adaptive Sampling**: Adjusts based on image size
- **K-means Clustering**: For dominant color extraction
- **Linear Interpolation**: For smooth gradient generation
- **HSV Enhancement**: Optional color saturation/brightness adjustment