struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct ParallaxUniform {
    translation_ndc: vec2<f32>,
    scale: f32,
    rotation_rad: f32,
    opacity: f32,
    _pad: f32,
    crop_scale: vec2<f32>,
    crop_offset: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: ParallaxUniform;
@group(1) @binding(0) var layer_tex: texture_2d<f32>;
@group(1) @binding(1) var layer_sampler: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let c = cos(u.rotation_rad);
    let s = sin(u.rotation_rad);
    let scaled = in.position * u.scale;
    let rotated = vec2<f32>(
        scaled.x * c - scaled.y * s,
        scaled.x * s + scaled.y * c,
    );

    out.position = vec4<f32>(rotated + u.translation_ndc, 0.0, 1.0);

    // Apply the same crop transform as the main shader:
    // crop_offset is in UV space, crop_scale > 1 means we stretch UVs
    // to effectively crop the texture (keep aspect ratio).
    out.uv = in.uv * u.crop_scale + u.crop_offset;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(layer_tex, layer_sampler, in.uv);
    return vec4<f32>(sample.rgb, sample.a * u.opacity);
}
