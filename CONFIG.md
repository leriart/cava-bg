# cava-bg Configuration Reference

<!-- Auto-generated from src/app_config.rs. Do not edit by hand. -->

> **Type notation:** `?T` = optional `T`, `T[]` = array of `T`, `map[K, V]` = table keyed by `K` with value type `V`.
>
> **Legend:** `—` indicates that the field is optional and has no default value (it will be `None` or absent in TOML).

## Table of Contents

### Configuration Sections

- [`Config`](#config)
- [`[global]`](#configoverride)
- [`[output."<name>"]`](#outputoverrideconfig)
- [`[general]`](#generalconfig)
- [`[audio]`](#audioconfig)
- [`[colors]`](#colorsconfig)
- [`[display]`](#displayconfig)
- [`[smoothing]`](#smoothingconfig)
- [`[hidden_image]`](#hiddenimageconfig)
- [`[layers]`](#layersconfig)
- [`[wallpaper]`](#wallpaperconfig)
- [`[parallax]`](#parallaxconfig)
- [`[performance]`](#performanceconfig)
- [`[xray_mask]`](#xraymaskconfig)
- [`[advanced]`](#advancedconfig)
- [`[xray]`](#xrayconfig)

### Data Types

- [`GradientDirection`](#gradientdirection)
- [`ColorExtractionMode`](#colorextractionmode)
- [`VisualizationMode`](#visualizationmode)
- [`BarShape`](#barshape)
- [`BarShapeConfig`](#barshapeconfig)
- [`Position`](#position)
- [`LayerChoice`](#layerchoice)
- [`ConfigColor`](#configcolor)
- [`HexColorConfig`](#hexcolorconfig)
- [`LayerConfig`](#layerconfig)
- [`LayerSourceConfig`](#layersourceconfig)
- [`LayerSourceType`](#layersourcetype)
- [`LayerSyncConfig`](#layersyncconfig)
- [`ParallaxMode`](#parallaxmode)
- [`ProfileSource`](#profilesource)
- [`ParallaxPreset`](#parallaxpreset)
- [`ParallaxMouseConfig`](#parallaxmouseconfig)
- [`ParallaxPerformanceConfig`](#parallaxperformanceconfig)
- [`FrequencyZone`](#frequencyzone)
- [`AudioResponseCurve`](#audioresponsecurve)
- [`ParallaxLayerAudioConfig`](#parallaxlayeraudioconfig)
- [`LayerAudioTransformConfig`](#layeraudiotransformconfig)
- [`LayerMouseReactivityConfig`](#layermousereactivityconfig)
- [`ParallaxEffectType`](#parallaxeffecttype)
- [`ParallaxEffectConfig`](#parallaxeffectconfig)
- [`ParallaxLayerConfig`](#parallaxlayerconfig)
- [`DropShadowConfig`](#dropshadowconfig)
- [`LayerAnimationConfig`](#layeranimationconfig)
- [`AnimationType`](#animationtype)
- [`IdleModeConfig`](#idlemodeconfig)
- [`VideoDecoderPerfConfig`](#videodecoderperfconfig)
- [`MaskComputeMode`](#maskcomputemode)
- [`XrayPerformanceConfig`](#xrayperformanceconfig)
- [`PerformanceTelemetryConfig`](#performancetelemetryconfig)
- [`HiddenImageEffect`](#hiddenimageeffect)
- [`PaletteType`](#palettetype)
- [`BlendMode`](#blendmode)
- [`XRayAnimationType`](#xrayanimationtype)

---

## Configuration Sections

## `Config`

Top-level configuration for cava-bg. Each sub-table controls a specific subsystem (audio, display, colors, etc.). Per-output overrides can be defined under `[output."<name>"]`.

| Section / Field | Type | Description |
| --- | --- | --- |
| `general` | [[general]](#generalconfig) | General behaviour: framerate, background colour, auto-sensitivity. |
| `audio` | [[audio]](#audioconfig) | Audio visualiser bar parameters: count, size, colours, shape, smoothing. |
| `colors` | [[colors]](#colorsconfig) | Colour palette, gradient settings, wallpaper colour extraction. |
| `display` | [[display]](#displayconfig) | Output positioning, anchoring, margins, layer choice. |
| `smoothing` | [[smoothing]](#smoothingconfig) | Smoothing overrides forwarded to the cava subprocess. |
| `hidden_image` | ?[[hidden_image]](#hiddenimageconfig) | Obsolete: hidden image overlay (replaced by `[layers]`). |
| `layers` | ?[[layers]](#layersconfig) | Image / video overlay layers with X-Ray reveal support. |
| `parallax` | [[parallax]](#parallaxconfig) | Parallax scrolling layers (audio- and mouse-reactive). |
| `wallpaper` | [[wallpaper]](#wallpaperconfig) | Wallpaper auto-detection from desktop environments. |
| `xray_mask` | [[xray_mask]](#xraymaskconfig) | X-Ray mask effect parameters (intensity, gamma, blend). |
| `xray` | [[xray]](#xrayconfig) | Legacy X-Ray effect configuration. |
| `performance` | [[performance]](#performanceconfig) | Performance tuning: vsync, threading, idle mode, telemetry. |
| `advanced` | [[advanced]](#advancedconfig) | Advanced / debugging settings. |
| `global` | ?[[global]](#configoverride) | Default overrides applied to all outputs (before per-output rules). |
| `output."<name>"` | [[output."<name>"]](#outputoverrideconfig) | Per-output overrides keyed by output name / connector / index. |

<a name="configoverride"></a>
## `[global]` (ConfigOverride)

Partial config mirror used for `[global]` and per-output overrides. Every field is optional; only sections to override need to be provided.

| Section / Field | Type | Description |
| --- | --- | --- |
| `general` | ?[[general]](#generalconfig) | *No description* |
| `audio` | ?[[audio]](#audioconfig) | *No description* |
| `colors` | ?[[colors]](#colorsconfig) | *No description* |
| `display` | ?[[display]](#displayconfig) | *No description* |
| `smoothing` | ?[[smoothing]](#smoothingconfig) | *No description* |
| `hidden_image` | ?[[hidden_image]](#hiddenimageconfig) | *No description* |
| `layers` | ?[[layers]](#layersconfig) | *No description* |
| `parallax` | ?[[parallax]](#parallaxconfig) | *No description* |
| `wallpaper` | ?[[wallpaper]](#wallpaperconfig) | *No description* |
| `xray_mask` | ?[[xray_mask]](#xraymaskconfig) | *No description* |
| `xray` | ?[[xray]](#xrayconfig) | *No description* |
| `performance` | ?[[performance]](#performanceconfig) | *No description* |
| `advanced` | ?[[advanced]](#advancedconfig) | *No description* |

<a name="outputoverrideconfig"></a>
## `[output."<name>"]` (OutputOverrideConfig)

Per-output configuration override. Matched against runtime output descriptors by name, connector, or index.

| Section / Field | Type | Description |
| --- | --- | --- |
| `enabled` | ?boolean | Whether this output is enabled (omit = inherited). |
| `name` | ?string | Exact output name or wildcard pattern. |
| `connector` | ?string | Connector name (e.g. "DP-1", "HDMI-A-1"). |
| `index` | ?unsigned integer | Output index (0-based). |
| `config` | [[global]](#configoverride) | Overrides inherited from the parent config. |

<a name="generalconfig"></a>
## `[general]` (GeneralConfig)

General application behaviour.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `framerate` | unsigned integer | `60` | Target framerate for the visualiser. |
| `background_color` | [ConfigColor](#configcolor) | ` { hex = "#000000", alpha = 0.0 }` | Background colour behind the bars (hex + optional alpha). |
| `autosens` | ?boolean | `—` | Enable cava auto-sensitivity (omit = use cava default). |
| `sensitivity` | ?float | `—` | cava sensitivity multiplier (omit = use cava default). |
| `preferred_outputs` | string[] | `[]` | List of output names preferred for showing the visualiser. |
| `dynamic_colors` | boolean | `false` | Dynamically cycle colours based on audio amplitude. |
| `corner_radius` | float | `0.0` | Corner radius for the visualiser background. |
| `disable_audio` | boolean | `false` | Disable audio capture (currently a no-op — cava always starts). |

<a name="audioconfig"></a>
## `[audio]` (AudioConfig)

Audio visualiser bar configuration. Controls the appearance, shape, and behaviour of the audio bars rendered as a Wayland background layer. Also stores legacy gradient / glow fields for backward compatibility.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `bar_count` | unsigned integer | `76` | Number of bars. |
| `bar_width` | float | `5.0` | Width of each bar in logical pixels. |
| `bar_spacing` | float | `2.0` | Horizontal spacing between bars in logical pixels. |
| `gap` | float | `0.1` | Inter-bar spacing as a fraction of bar width (0..1). |
| `bar_alpha` | float | `1.0` | Opacity of every bar (0..1). |
| `height_scale` | float | `1.0` | Overall height multiplier for bars. |
| `smoothing` | float | `0.8` | Visual smoothing factor (0..1) — not currently forwarded to cava. |
| `bar_color` | [ConfigColor](#configcolor) | ` { hex = "#ff00ff", alpha = 1.0 }` | Solid colour used for all bars when colour extraction is disabled. |
| `max_bar_height` | float | `100.0` | Maximum bar height in logical pixels. |
| `min_bar_height` | float | `0.0` | Minimum bar height in logical pixels. |
| `mirror_bars` | boolean | `false` | Obsolete: mirror bars vertically (use `visualization_mode = "MirrorBars"`). |
| `bar_shape` | [BarShape](#barshape) | `"Rectangle"` | Bar shape: Rectangle, Circle, Triangle, or Line. |
| `corner_radius` | float | `6.0` | Corner radius for rounded bars, in logical pixels. |
| `corner_segments` | unsigned integer | `8` | Number of segments used for rounded corners (higher = smoother). |
| `extract_colors_from_wallpaper` | boolean | `false` | Obsolete: does not take effect — use [`ColorsConfig::extract_from_wallpaper`] instead. |
| `color_extraction_mode` | [ColorExtractionMode](#colorextractionmode) | `"Dominant"` | Wallpaper colour extraction method: Dominant, Vibrant, or Palette. |
| `visualization_mode` | [VisualizationMode](#visualizationmode) | `"Bars"` | Visualization style: Bars, Waveform, Blocks, MirrorBars, InvertedBars, Spectrum, Ring. |
| `polygon_sides` | unsigned integer | `6` | Number of polygon sides for certain visualization modes. |
| `show_visualizer` | boolean | `true` | Show the audio visualiser overlay. |
| `radial_inner_radius` | float | `30.0` | Inner radius for Ring mode, in logical pixels. |
| `radial_sweep_angle` | float | `360.0` | Sweep angle for Ring mode in degrees. |
| `waveform_line_width` | float | `2.0` | Line width for Waveform mode, in logical pixels. |
| `waveform_smoothness` | float | `0.5` | Smoothness factor for Waveform mode (0..1). |
| `block_size` | float | `10.0` | Block size for Blocks mode, in logical pixels. |
| `block_spacing` | float | `2.0` | Spacing between blocks in Blocks mode, in logical pixels. |
| `spiral_turns` | float | `2.0` | Number of turns for Spiral visualization mode (1.0 = single revolution). |
| `mirror_gap` | float | `0.04` | Gap between mirrored halves in MirrorBars mode (0..1, fraction of the vertical extent reserved as a horizontal gutter at the center line). |

<a name="colorsconfig"></a>
## `[colors]` (ColorsConfig)

Colour settings: palette, gradient, wallpaper extraction.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `extract_from_wallpaper` | boolean | `false` | Extract colours from the current wallpaper instead of using the palette. |
| `extraction_mode` | [ColorExtractionMode](#colorextractionmode) | `"Dominant"` | Colour extraction method when `extract_from_wallpaper` is true. |
| `palette` | [float; 4][] | `[0.58 , 0.89 , 0.84 , 1.0] , [0.45 , 0.78 , 0.93 , 1.0] , ...` | Colour palette as a list of RGBA arrays. |
| `gradient_direction` | [GradientDirection](#gradientdirection) | `"BottomToTop"` | Direction of the colour gradient applied to bars. |
| `use_gradient` | boolean | `true` | Apply gradient colours across bars. |

<a name="displayconfig"></a>
## `[display]` (DisplayConfig)

Output positioning and sizing.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `position` | [Position](#position) | `"Fill"` | Positioning mode: Fill, Center, Top, Bottom, Left, Right, Custom. |
| `anchor_top` | boolean | `true` | Anchor the visualiser to the top edge. |
| `anchor_bottom` | boolean | `true` | Anchor the visualiser to the bottom edge. |
| `anchor_left` | boolean | `true` | Anchor the visualiser to the left edge. |
| `anchor_right` | boolean | `true` | Anchor the visualiser to the right edge. |
| `width` | unsigned integer | `0` | Custom width in logical pixels (0 = auto). |
| `height` | unsigned integer | `0` | Custom height in logical pixels (0 = auto). |
| `margin_top` | unsigned integer | `0` | Top margin in logical pixels. |
| `margin_bottom` | unsigned integer | `0` | Bottom margin in logical pixels. |
| `margin_left` | unsigned integer | `0` | Left margin in logical pixels. |
| `margin_right` | unsigned integer | `0` | Right margin in logical pixels. |
| `layer` | [LayerChoice](#layerchoice) | `"Bottom"` | Wayland layer: Background, Bottom, Top, Overlay. |
| `opacity` | float | `1.0` | Layer opacity (0..1). |
| `scale_with_resolution` | boolean | `false` | Scale the visualiser with output resolution changes. |

<a name="smoothingconfig"></a>
## `[smoothing]` (SmoothingConfig)

Smoothing parameters forwarded to the cava subprocess. These are passed verbatim to cava's `[smoothing]` section.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `monstercat` | ?float | `—` | Monstercat smoothing factor (0..1). |
| `waves` | ?integer | `—` | Waves smoothing setting (integer). |
| `noise_reduction` | ?float | `—` | Noise reduction factor. |

<a name="hiddenimageconfig"></a>
## `[hidden_image]` (HiddenImageConfig)

Obsolete hidden-image overlay (replaced by `[layers]`).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `use_wallpaper` | boolean | `false` | Use the current wallpaper as the hidden image. |
| `path` | ?string | `—` | Path to the hidden image file. |
| `effect` | [HiddenImageEffect](#hiddenimageeffect) | Default | Visual effect applied to the hidden image. |
| `blend_mode` | [BlendMode](#blendmode) | Default | Blend mode for the reveal effect. |
| `wallpapers_dir` | ?string | `—` | Directory containing wallpapers for detection. |

<a name="layersconfig"></a>
## `[layers]` (LayersConfig)

Image / video layer management with X-Ray reveal support — layer system not yet wired to the renderer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `base` | [LayerConfig](#layerconfig) | Required | Base (background) layer configuration. |
| `reveal` | [LayerConfig](#layerconfig) | Required | Reveal (foreground) layer configuration. |
| `sync` | [LayerSyncConfig](#layersyncconfig) | Default | Wallpaper synchronisation settings for fingerprint-based matching. |

<a name="wallpaperconfig"></a>
## `[wallpaper]` (WallpaperConfig)

Wallpaper detection and synchronisation settings.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `auto_detect_wallpaper` | boolean | `true` | Auto-detect the current wallpaper from the desktop environment. |
| `xray_layers_dir` | ?path | `—` | Custom directory for X-Ray layer images. |
| `wallpapers_dir` | ?path | `—` | Custom directory containing wallpapers for detection. |
| `sync_interval_seconds` | float | `10.0` | Interval (in seconds) between wallpaper checks. |

<a name="parallaxconfig"></a>
## `[parallax]` (ParallaxConfig)

Parallax scrolling layer system (audio- and mouse-reactive backgrounds).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable parallax effects. |
| `mode` | [ParallaxMode](#parallaxmode) | `"Hybrid"` | Parallax mode: AudioReactive, MouseReactive, Animated, Hybrid (not currently read by the renderer). |
| `enable_3d_depth` | boolean | `false` | Enable 3D-like depth effect for parallax layers. |
| `mouse` | [ParallaxMouseConfig](#parallaxmouseconfig) | `{ enabled = true, global_tracking = true, per_output_tracking = true, range = 1.0, sensitivity = 0.35 }` | Mouse tracking settings for mouse-reactive parallax. |
| `performance` | [ParallaxPerformanceConfig](#parallaxperformanceconfig) | `{ disable_under_load = false, frame_time_budget_ms = 18.0, lazy_load_assets = true, pause_on_idle = true }` | Performance tuning for parallax rendering. |
| `preset` | ?[ParallaxPreset](#parallaxpreset) | `—` | Quick preset override: SoftDepth, AudioPulse, Cinematic. |
| `layers` | [ParallaxLayerConfig](#parallaxlayerconfig)[] | `[]` | List of parallax layer definitions. |
| `show_visualizer` | boolean | `true` | Show the audio visualiser on top of parallax layers. |
| `visualizer_as_parallax_layer` | boolean | `false` | Treat the visualiser as a parallax layer. |
| `visualizer_layer_index` | unsigned integer | `0` | Z-index of the visualiser when treated as a parallax layer. |
| `profiles_dir` | ?path | `—` | Directory containing saved parallax profiles. |
| `profile_source` | [ProfileSource](#profilesource) | `"wallpaper"` | Profile source mode: normal or from wallpaper auto-detection. |
| `active_profile` | ?string | `—` | Name of the currently active parallax profile. |

<a name="performanceconfig"></a>
## `[performance]` (PerformanceConfig)

Performance and optimisation settings.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `vsync` | boolean | `true` | Enable V-Sync (limits to monitor refresh rate). |
| `multi_threaded_decode` | boolean | `true` | Decode video frames on worker threads. |
| `idle_mode` | [IdleModeConfig](#idlemodeconfig) | `{ audio_threshold = 0.015, enabled = true, exit_transition_ms = 250, idle_fps = 10, timeout_seconds = 5.0 }` | Idle mode to save resources when no audio is playing. |
| `video_decoder` | [VideoDecoderPerfConfig](#videodecoderperfconfig) | `{ auto_shutdown = true, debug_telemetry = false, lazy_init = true, pause_on_idle = true, shutdown_after_seconds = 20.0 }` | Video decoder performance tuning. |
| `xray` | [XrayPerformanceConfig](#xrayperformanceconfig) | `{ generate_mipmaps = true, mask_compute_mode = "Auto", prescale_max_dimension = 2048 }` | X-Ray mask performance settings. |
| `telemetry` | [PerformanceTelemetryConfig](#performancetelemetryconfig) | `{ enabled = false, log_interval_seconds = 5, metrics_window = 240 }` | Performance telemetry and metrics. |

<a name="xraymaskconfig"></a>
## `[xray_mask]` (XrayMaskConfig)

X-Ray mask effect parameters controlling reveal intensity and appearance.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `intensity` | float | `0.8` | Mask intensity (0..1). |
| `gamma` | float | `1.2` | Gamma correction for the mask. |
| `opacity` | float | `1.0` | Mask opacity (0..1). |
| `blend_mode` | [BlendMode](#blendmode) | `"Normal"` | Blend mode for the mask. |
| `xray_background_color` | ?[float; 4] | `—` | Custom background colour for the mask overlay (RGBA). |
| `use_background` | boolean | `false` | Use [`xray_background_color`] as the mask background. |

<a name="advancedconfig"></a>
## `[advanced]` (AdvancedConfig)

Advanced / debugging settings.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `verbose_logging` | boolean | `false` | Enable verbose logging output. |
| `frame_rate_limit` | unsigned integer | `60` | Hard framerate cap for the render loop. |
| `layer_cache_size` | unsigned integer | `5` | Maximum number of layers to keep cached in memory. |

<a name="xrayconfig"></a>
## `[xray]` (XRayConfig)

X-Ray effect configuration (base + reveal image compositing).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable the X-Ray effect. |
| `intensity` | float | `0.8` | X-Ray intensity (0..1). |
| `blend_mode` | [BlendMode](#blendmode) | `"Normal"` | Blend mode for the X-Ray overlay. |
| `base_layer_path` | string | `empty string` | Path to the base layer image. |
| `reveal_layer_path` | string | `empty string` | Path to the reveal layer image. |
| `auto_detect` | boolean | `true` | Auto-detect layers from wallpaper. |
| `use_background_color` | boolean | `false` | Use a solid background colour instead of the blurred wallpaper. |
| `background_color` | [float; 4] | `[0.0, 0.0, 0.0, 1.0]` | Background colour when `use_background_color` is true. |
| `images_dir` | ?string | `—` | Directory containing X-Ray images (for auto-detection). |
| `animation_enabled` | boolean | `false` | Enable X-Ray animation. |
| `animation_type` | [XRayAnimationType](#xrayanimationtype) | `"None"` | Animation type for the X-Ray effect. |
| `animation_speed` | float | `1.0` | Animation speed multiplier. |
| `audio_sensitivity` | float | `1.0` | Audio sensitivity for audio-reactive animation. |
| `auto_detect_wallpaper_fps` | boolean | `true` | Sync X-Ray animation FPS with wallpaper detection rate. |

---

## Data Types

## `GradientDirection`

Direction of the colour gradient (legacy).

| Variant | Description |
| --- | --- |
| `TopToBottom` | Gradient from top to bottom. |
| `BottomToTop` | Gradient from bottom to top. (Default) |
| `LeftToRight` | Gradient from left to right. |
| `RightToLeft` | Gradient from right to left. |

## `ColorExtractionMode`

Method used to extract colours from the wallpaper.

| Variant | Description |
| --- | --- |
| `Dominant` | Use the most dominant colour from the wallpaper. (Default) |
| `Vibrant` | Use the most vibrant colour from the wallpaper. |
| `Palette` | Extract a full palette from the wallpaper. |

## `VisualizationMode`

Visualization style for the audio bars.

| Variant | Description |
| --- | --- |
| `Bars` | Standard vertical bars growing from the bottom. (Default) |
| `Waveform` | Smooth waveform line following the audio amplitude. |
| `Blocks` | Rectangular blocks arranged in a grid pattern. |
| `MirrorBars` | Bars mirrored vertically: each bar extends symmetrically up and down from the horizontal center line. Ideal for music players / DJ visuals. |
| `InvertedBars` | Bars hanging from the top edge instead of growing from the bottom. |
| `Spectrum` | Smooth thick line connecting the top of each bar — a classic "spectrum analyzer" curve. |
| `Ring` | Continuous filled ring whose thickness is modulated by the bin amplitudes (a polar variant of Bars). |

## `BarShape`

Shape of individual bars.

| Variant | Description |
| --- | --- |
| `Rectangle` | Standard rectangular bars. (Default) |
| `Circle` | Circular bars. |
| `Triangle` | Triangular bars. |
| `Line` | Thin line bars. |

## `BarShapeConfig`

Per-bar shape configuration (extended variant).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `shape` | [BarShape](#barshape) | `"Rectangle"` | Bar shape variant. |
| `corner_radius` | float | `6.0` | Corner radius for rounded bars, in logical pixels. |
| `corner_segments` | unsigned integer | `8` | Number of segments for rounded corners. |

## `Position`

Positioning mode for the visualiser on screen.

| Variant | Description |
| --- | --- |
| `Fill` | Fill the entire output. (Default) |
| `Center` | Center on the output. |
| `Top` | Align to the top edge. |
| `Bottom` | Align to the bottom edge. |
| `Left` | Align to the left edge. |
| `Right` | Align to the right edge. |
| `Custom` | Custom position (requires manual anchor configuration). |

## `LayerChoice`

Wayland layer at which the visualiser is rendered.

| Variant | Description |
| --- | --- |
| `Background` | Below the wallpaper / background. |
| `Bottom` | Normal bottom layer. (Default) |
| `Top` | Above most windows. |
| `Overlay` | Always-on-top overlay. |

## `ConfigColor`

A colour value specified either as a hex string or as a struct with hex + alpha.

| Variant | Fields | Description |
| --- | --- | --- |
| string | string | Simple hex colour string, e.g. `"#ff00ff"`. |
| [HexColorConfig](#hexcolorconfig) | [HexColorConfig](#hexcolorconfig) | Hex colour with a separate alpha channel, expressed as an inline table `{ hex = "...", alpha = ... }`. |

## `HexColorConfig`

Hex colour with optional alpha.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `hex` | string | Required | Hex colour string, e.g. `"#ff00ff"`. |
| `alpha` | ?float | `—` | Alpha channel (0..1). Defaults to 1.0 when absent. |

## `LayerConfig`

Configuration for a single image/video layer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | Enable this layer. |
| `source` | [LayerSourceConfig](#layersourceconfig) | Required | Source configuration (type, path, looping). |
| `fit` | string | `"cover"` | CSS-like fit mode: "cover", "contain", "fill", "none (only "cover" is implemented). |
| `opacity` | float | `1.0` | Layer opacity (0..1). |
| `blend_mode` | [BlendMode](#blendmode) | Default | Blend mode for compositing. |
| `max_buffered_frames` | unsigned integer | `6` | Maximum number of buffered video frames. |
| `frame_cache_size` | unsigned integer | `120` | Frame cache size for decoded video. |

## `LayerSourceConfig`

Source specification for a layer (image, video, or GIF).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `type` | [LayerSourceType](#layersourcetype) | Required | Source type: "static", "video", or "gif". |
| `path` | string | Required | File path to the source asset. |
| `looping` | boolean | `true` | Loop the video/GIF playback. |

## `LayerSourceType`

Supported layer source types.

| Variant | Description |
| --- | --- |
| `static` | Static image (PNG, JPEG, WebP, etc.). |
| `video` | Video file (MP4, WebM, MKV, etc.). |
| `gif` | Animated GIF. |

## `LayerSyncConfig`

Wallpaper synchronisation settings for fingerprint-based layer matching.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `sync_with_wallpaper` | boolean | `true` | Automatically sync layers with the detected wallpaper. |
| `fingerprint_search_window` | unsigned integer | `90` | Search window size for fingerprint matching (in wallpapers) — not yet implemented. |
| `fingerprint_min_confidence` | float | `0.55` | Minimum confidence score (0..1) for fingerprint matching — not yet implemented. |

## `ParallaxMode`

Parallax reactivity mode.

| Variant | Description |
| --- | --- |
| `AudioReactive` | Layers react to audio amplitude and frequency. (Default) |
| `MouseReactive` | Layers move in response to mouse position. |
| `Animated` | Layers animate automatically without input. |
| `Hybrid` | Combination of audio-reactive and mouse-reactive modes. |

## `ProfileSource`

Source mode for parallax profile selection.

| Variant | Description |
| --- | --- |
| `normal` | Use a manually specified profile. |
| `wallpaper` | Auto-detect profile from the current wallpaper name. (Default) |

## `ParallaxPreset`

Quick parallax preset that overrides individual layer settings.

| Variant | Description |
| --- | --- |
| `SoftDepth` | Subtle depth-of-field style. |
| `AudioPulse` | Layers pulse with the audio beat. |
| `Cinematic` | Cinematic widescreen-style parallax. |

## `ParallaxMouseConfig`

Mouse tracking settings for mouse-reactive parallax layers.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | Enable mouse tracking. |
| `sensitivity` | float | `0.35` | Mouse sensitivity (cursor movement → layer displacement). |
| `range` | float | `1.0` | Maximum displacement range in logical pixels. |
| `global_tracking` | boolean | `true` | Track mouse globally across all outputs. |
| `per_output_tracking` | boolean | `true` | Track mouse independently per output. |

## `ParallaxPerformanceConfig`

Performance tuning for parallax rendering.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `disable_under_load` | boolean | `false` | Disable parallax under system load. |
| `frame_time_budget_ms` | float | `18.0` | Maximum frame time budget in ms. |
| `lazy_load_assets` | boolean | `true` | Lazily load layer assets only when visible. |
| `pause_on_idle` | boolean | `true` | Pause parallax animation when system is idle. |

## `FrequencyZone`

Frequency band selection for audio-reactive layers.

| Variant | Description |
| --- | --- |
| `FullSpectrum` | React to the full frequency spectrum. (Default) |
| `Low` | Low frequencies only (bass). |
| `Mid` | Mid-range frequencies. |
| `High` | High frequencies (treble). |

## `AudioResponseCurve`

Response curve shape for audio-reactive animation.

| Variant | Description |
| --- | --- |
| `Linear` | Linear response. |
| `Smooth` | Smooth, eased response. (Default) |
| `Exponential` | Exponential (more sensitive to loud sounds). |
| `Punchy` | Punchy, fast-attack response. |

## `ParallaxLayerAudioConfig`

Audio reactivity settings for a single parallax layer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable audio reactivity for this layer. |
| `frequency_zone` | [FrequencyZone](#frequencyzone) | `"FullSpectrum"` | Frequency zone to react to. |
| `response_curve` | [AudioResponseCurve](#audioresponsecurve) | `"Smooth"` | Response curve shape. |
| `amplitude_sensitivity` | float | `0.5` | How strongly amplitude affects the layer. |
| `frequency_sensitivity` | float | `0.5` | How strongly frequency changes affect the layer. |
| `transform` | [LayerAudioTransformConfig](#layeraudiotransformconfig) | `{ rotate = false, rotation_amount = 6.0, scale = false, scale_amount = 0.08, shift = true, shift_amount = 28.0 }` | Audio-driven transform effects (shift, scale, rotate). |

## `LayerAudioTransformConfig`

Audio-driven transform effects.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `shift` | boolean | `true` | Enable horizontal / vertical shifting. |
| `scale` | boolean | `false` | Enable scaling. |
| `rotate` | boolean | `false` | Enable rotation. |
| `shift_amount` | float | `28.0` | Shift displacement in logical pixels. |
| `scale_amount` | float | `0.08` | Scale factor. |
| `rotation_amount` | float | `6.0` | Rotation amount in degrees. |

## `LayerMouseReactivityConfig`

Mouse reactivity settings for a single parallax layer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | Enable mouse tracking. |
| `sensitivity` | float | `0.35` | Mouse sensitivity. |
| `max_offset` | [float; 2] | `[32.0, 32.0]` | Maximum displacement [x, y] in logical pixels. |

## `ParallaxEffectType`

Type of cava visualiser effect used as a parallax layer source.

| Variant | Description |
| --- | --- |
| `CavaBars` | Standard bar visualiser. (Default) |
| `CavaWave` | Waveform-style visualiser. |
| `CavaRadial` | Radial / ring visualiser. |

## `ParallaxEffectConfig`

Configuration for a cava effect used as a parallax layer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable this effect. |
| `effect_type` | [ParallaxEffectType](#parallaxeffecttype) | `"CavaBars"` | Visualiser effect type. |
| `bars` | unsigned integer | `48` | Number of bars in the effect. |
| `tint` | [float; 4] | `[0.75, 0.85, 1.0, 0.95]` | Tint colour applied to the effect (RGBA). |
| `gap` | float | `0.0` | Gap between effect elements. |
| `height_scale` | float | `1.0` | Height scale for the effect. |

## `ParallaxLayerConfig`

A single parallax layer with its source, position, and reactivity settings.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | `empty string` | Layer display name. |
| `enabled` | boolean | `true` | Enable this layer. |
| `source` | string | `empty string` | Source path or effect reference (e.g. "effect:cava-bars"). |
| `layer_type` | ?[LayerSourceType](#layersourcetype) | `—` | Source type (auto-detected from extension if not set). |
| `effect` | ?[ParallaxEffectConfig](#parallaxeffectconfig) | `—` | Effect configuration when source starts with "effect:". |
| `z_index` | integer | `0` | Z-index for layer ordering (higher = on top). |
| `depth` | float | `0.5` | Parallax depth factor (0..1). |
| `opacity` | float | `1.0` | Layer opacity (0..1). |
| `blend_mode` | [BlendMode](#blendmode) | `"Normal"` | Blend mode for compositing. |
| `offset` | [float; 2] | `[0.0, 0.0]` | Base positional offset [x, y] in logical pixels. |
| `parallax_speed` | float | `0.5` | Parallax scroll speed multiplier. |
| `audio` | [ParallaxLayerAudioConfig](#parallaxlayeraudioconfig) | `{ amplitude_sensitivity = 0.5, enabled = false, frequency_sensitivity = 0.5, frequency_zone = "FullSpectrum", response_curve = "Smooth", transform = { rotate = false, rotation_amount = 6.0, scale = false, scale_amount = 0.08, shift = true, shift_amount = 28.0 } }` | Audio reactivity settings. |
| `mouse` | [LayerMouseReactivityConfig](#layermousereactivityconfig) | `{ enabled = true, max_offset = [32.0, 32.0], sensitivity = 0.35 }` | Mouse reactivity settings. |
| `react_to_audio` | boolean | `false` | Obsolete: enable audio reaction (use audio.enabled). |
| `audio_reaction_intensity` | float | `0.5` | Obsolete: audio reaction intensity (use audio.amplitude_sensitivity). |
| `react_to_mouse` | boolean | `true` | Obsolete: enable mouse reaction (use mouse.enabled). |
| `mouse_depth_factor` | float | `0.8` | Obsolete: mouse depth factor (use mouse.sensitivity). |
| `animation` | ?[LayerAnimationConfig](#layeranimationconfig) | `—` | Optional animation config for this layer. |
| `drop_shadow` | ?[DropShadowConfig](#dropshadowconfig) | `—` | Optional drop shadow config. |

## `DropShadowConfig`

Drop shadow effect for parallax layers.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `color` | [float; 4] | Required | Shadow colour as RGBA. |
| `offset` | [float; 2] | Required | Shadow offset [x, y] in logical pixels. |
| `blur_radius` | float | Required | Blur radius in logical pixels. |
| `spread` | float | `0.0` | Spread of the shadow. |

## `LayerAnimationConfig`

Animation configuration for a parallax layer.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable animation. |
| `type` | [AnimationType](#animationtype) | Default | Animation type. |
| `speed` | float | `0.5` | Animation speed. |
| `amplitude` | float | `10.0` | Animation amplitude. |

## `AnimationType`

Animation type for a parallax layer.

| Variant | Description |
| --- | --- |
| `Float` | Floating / bobbing motion. (Default) |
| `Rotate` | Rotation animation. |
| `Scale` | Scaling (pulsing size). |
| `Pulse` | Opacity pulse. |
| `Wiggle` | Wiggling motion. |

## `IdleModeConfig`

Idle mode: reduces rendering when audio is silent for a period.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | Enable idle mode. |
| `audio_threshold` | float | `0.015` | RMS audio threshold below which the system is considered idle. |
| `timeout_seconds` | float | `5.0` | Seconds of silence before entering idle mode. |
| `idle_fps` | unsigned integer | `10` | Target framerate while idle. |
| `exit_transition_ms` | unsigned integer | `250` | Transition time (ms) when exiting idle mode. |

## `VideoDecoderPerfConfig`

Video decoder performance configuration.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `lazy_init` | boolean | `true` | Lazily initialise the decoder on first use. |
| `auto_shutdown` | boolean | `true` | Automatically shut down the decoder when not in use. |
| `shutdown_after_seconds` | float | `20.0` | Seconds of inactivity before decoder shutdown. |
| `pause_on_idle` | boolean | `true` | Pause decoder when system is idle. |
| `debug_telemetry` | boolean | `false` | Log decoder performance metrics. |

## `MaskComputeMode`

Compute mode for X-Ray mask generation.

| Variant | Description |
| --- | --- |
| `Auto` | Automatically select CPU or GPU. (Default) |
| `Cpu` | Force CPU-based mask computation. |
| `Gpu` | Force GPU-based mask computation. |

## `XrayPerformanceConfig`

Performance settings for X-Ray mask rendering.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `prescale_max_dimension` | unsigned integer | `2048` | Maximum image dimension (px) for pre-scaling before mask computation. |
| `generate_mipmaps` | boolean | `true` | Generate mipmaps for mask images. |
| `mask_compute_mode` | [MaskComputeMode](#maskcomputemode) | `"Auto"` | Mask computation mode (Auto / CPU / GPU). |

## `PerformanceTelemetryConfig`

Performance telemetry / metrics configuration.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `false` | Enable performance metrics collection. |
| `metrics_window` | unsigned integer | `240` | Number of frames in the rolling metrics window. |
| `log_interval_seconds` | unsigned integer | `5` | Interval (seconds) between telemetry log writes. |

## `HiddenImageEffect`

Visual effect applied to a hidden image layer.

| Variant | Fields | Description |
| --- | --- | --- |
| `None` |  | No effect. (Default) |
| `Grayscale` |  | Convert to grayscale. |
| `Invert` |  | Invert colours. |
| `Sepia` |  | Apply sepia tone. |
| `palette` | [PaletteType](#palettetype) | Apply a preset colour palette. |

## `PaletteType`

Preset colour palette for palette-based effects.

| Variant | Description |
| --- | --- |
| `Catppuccin` | Catppuccin Mocha palette. |
| `Nord` | Nord palette. |
| `Gruvbox` | Gruvbox palette. |
| `Solarized` | Solarized palette. |

## `BlendMode`

Compositing blend mode for layers.

| Variant | Description |
| --- | --- |
| `Reveal` | Reveal the base layer through the foreground (functionally identical to Normal in current renderer). (Default) |
| `Normal` | Normal alpha blending. |
| `Add` | Additive blending. |
| `Multiply` | Multiplicative blending. |
| `Screen` | Screen blending. |
| `Overlay` | Overlay blending. |

## `XRayAnimationType`

Animation type for the legacy X-Ray effect.

| Variant | Description |
| --- | --- |
| `None` | No animation. (Default) |
| `Fade` | Fade in/out. |
| `Pulse` | Pulsing effect. |
| `WaveReveal` | Wave reveal effect. |
| `AudioSync` | Audio-synchronised animation. |
| `WallpaperSync` | Synced with wallpaper changes. |

