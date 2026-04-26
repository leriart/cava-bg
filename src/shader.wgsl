struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.world_pos = (in.position + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

struct Uniforms {
    gradient_colors: array<vec4<f32>, 32>,
    params: vec4<f32>,       // x=colors_count, y=bar_alpha, z=use_hidden, w=effect_type
    window_size: vec2<f32>,
    texture_size: vec2<f32>,
    crop_scale: vec2<f32>,
    crop_offset: vec2<f32>,
    extra: vec4<f32>,        // x=gradient_dir(0-3), y=use_gradient(0/1), z=unused, w=unused
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var hidden_texture: texture_2d<f32>;
@group(1) @binding(1) var hidden_sampler: sampler;

fn apply_effect(color: vec3<f32>, effect: i32) -> vec3<f32> {
    switch effect {
        case 0: { return color; }
        case 1: {
            let gray = dot(color, vec3<f32>(0.299, 0.587, 0.114));
            return vec3<f32>(gray, gray, gray);
        }
        case 2: { return vec3<f32>(1.0) - color; }
        case 3: {
            let r = color.r * 0.393 + color.g * 0.769 + color.b * 0.189;
            let g = color.r * 0.349 + color.g * 0.686 + color.b * 0.168;
            let b = color.r * 0.272 + color.g * 0.534 + color.b * 0.131;
            return vec3<f32>(r, g, b);
        }
        case 4: {
            let lum = dot(color, vec3<f32>(0.299, 0.587, 0.114));
            let idx_float = lum * 7.99;
            if (idx_float < 1.0) { return vec3<f32>(0.878, 0.859, 0.953); }
            else if (idx_float < 2.0) { return vec3<f32>(0.961, 0.761, 0.906); }
            else if (idx_float < 3.0) { return vec3<f32>(0.953, 0.545, 0.659); }
            else if (idx_float < 4.0) { return vec3<f32>(0.922, 0.627, 0.675); }
            else if (idx_float < 5.0) { return vec3<f32>(0.796, 0.651, 0.969); }
            else if (idx_float < 6.0) { return vec3<f32>(0.537, 0.706, 0.980); }
            else if (idx_float < 7.0) { return vec3<f32>(0.455, 0.780, 0.925); }
            else { return vec3<f32>(0.580, 0.886, 0.835); }
        }
        default: { return color; }
    }
}

fn compute_crop_uv(world_pos: vec2<f32>) -> vec2<f32> {
    return world_pos * uniforms.crop_scale + uniforms.crop_offset;
}

fn get_gradient_pos(world_pos: vec2<f32>) -> f32 {
    let dir = i32(uniforms.extra.x);
    // 0=BottomToTop, 1=TopToBottom, 2=LeftToRight, 3=RightToLeft
    // world_pos is in [0,1] range where (0,0)=bottom-left, (1,1)=top-right
    switch dir {
        case 0 { return world_pos.y; }          // BottomToTop
        case 1 { return 1.0 - world_pos.y; }    // TopToBottom
        case 2 { return world_pos.x; }          // LeftToRight
        case 3 { return 1.0 - world_pos.x; }    // RightToLeft
        default { return world_pos.y; }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let colors_count = i32(uniforms.params.x);
    let bar_alpha = uniforms.params.y;
    let use_hidden_image = uniforms.params.z > 0.5;
    let effect_type = i32(uniforms.params.w);
    let use_gradient = uniforms.extra.y > 0.5;
    
    var base_color: vec4<f32>;
    if (colors_count == 1) {
        // Single color: use solid (either gradient or flat)
        base_color = uniforms.gradient_colors[0];
    } else if (use_gradient) {
        // Gradient mode: interpolate between colors in chosen direction
        let t = get_gradient_pos(in.world_pos);
        let findex = t * f32(colors_count - 1);
        let index = i32(findex);
        let step = findex - f32(index);
        var idx = index;
        if (idx == colors_count - 1) { idx = idx - 1; }
        base_color = mix(uniforms.gradient_colors[idx], uniforms.gradient_colors[idx + 1], step);
    } else {
        // Flat mode with multiple colors: cycle by world position (per-bar solid)
        base_color = uniforms.gradient_colors[0];
    }
    
    var final_color = base_color;
    if (use_hidden_image) {
        let uv = compute_crop_uv(in.world_pos);
        let tex_color = textureSample(hidden_texture, hidden_sampler, uv);
        let processed_rgb = apply_effect(tex_color.rgb, effect_type);
        final_color = vec4<f32>(processed_rgb, tex_color.a);
    }
    
    let alpha = final_color.a * bar_alpha;
    return vec4<f32>(final_color.rgb, alpha);
}
