struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    return out;
}

struct Uniforms {
    gradient_colors: array<vec4<f32>, 32>,
    colors_count: i32,
    _padding1: i32,
    _padding2: i32,
    _padding3: i32,
    window_size: vec2<f32>,
    _padding4: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(@builtin(position) coord: vec4<f32>) -> @location(0) vec4<f32> {
    let y = coord.y;
    let height = uniforms.window_size.y;
    if (uniforms.colors_count == 1) {
        return uniforms.gradient_colors[0];
    } else {
        let findex = (y * f32(uniforms.colors_count - 1)) / height;
        let index = i32(findex);
        let step = findex - f32(index);
        var idx = index;
        if (idx == uniforms.colors_count - 1) {
            idx = idx - 1;
        }
        return mix(uniforms.gradient_colors[idx], uniforms.gradient_colors[idx + 1], step);
    }
}