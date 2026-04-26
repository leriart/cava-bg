// Vector-based bar geometry builder.
//
// All shapes are generated as triangle lists (no indexing, no shader tricks)
// so the renderer can simply issue `draw(0..vertex_count, 0..1)`.
//
// Each vertex is `[pos.x, pos.y, uv.x, uv.y]` in NDC / [0,1] texture space,
// matching the vertex layout already expected by `shader.wgsl`.

use crate::app_config::BarShape;

#[inline(always)]
fn push_v(buf: &mut Vec<f32>, x: f32, y: f32, u: f32, v: f32) {
    buf.push(x);
    buf.push(y);
    buf.push(u);
    buf.push(v);
}

/// Build a non-indexed triangle list for a single bar.
///
/// Coordinates are in NDC: x0/x1 horizontal, y0/y1 vertical.
/// `radius_x` / `radius_y` are already converted from pixels into NDC
/// (so corners remain visually circular regardless of aspect ratio).
/// UVs are computed from the normalized [0,1] position inside the bar box.
#[inline]
pub fn build_bar(
    buf: &mut Vec<f32>,
    shape: BarShape,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius_x: f32,
    radius_y: f32,
    segments: u32,
    _polygon_sides: u32,
    line_only: bool,
) {
    if line_only || matches!(shape, BarShape::Line) {
        build_line_bar(buf, x0, y0, x1, y1);
        return;
    }

    match shape {
        BarShape::Rectangle => build_rectangle(buf, x0, y0, x1, y1),
        BarShape::Circle => {
            build_circle_top(buf, x0, y0, x1, y1, radius_x, radius_y, segments.max(8))
        }
        BarShape::Triangle => build_triangle_top(buf, x0, y0, x1, y1),
        BarShape::Line => build_line_bar(buf, x0, y0, x1, y1),
    }
}

/// Worst-case vertex count per bar for a given shape.
/// Used to pre-allocate buffers so the hot loop never reallocates.
pub fn vertices_per_bar(shape: BarShape, segments: u32) -> usize {
    match shape {
        BarShape::Rectangle => 6,
        BarShape::Circle => 6 + 3 * segments.max(8) as usize,
        BarShape::Triangle => 9,
        BarShape::Line => 6,
    }
}

// -- Rectangle -----------------------------------------------------------

#[inline]
fn build_rectangle(buf: &mut Vec<f32>, x0: f32, y0: f32, x1: f32, y1: f32) {
    // Two triangles: (x0,y0),(x1,y0),(x0,y1) and (x1,y0),(x1,y1),(x0,y1)
    // UV: y0 -> v=0 (bottom), y1 -> v=1 (top) ; x0 -> u=0 ; x1 -> u=1
    push_v(buf, x0, y0, 0.0, 0.0);
    push_v(buf, x1, y0, 1.0, 0.0);
    push_v(buf, x0, y1, 0.0, 1.0);
    push_v(buf, x1, y0, 1.0, 0.0);
    push_v(buf, x1, y1, 1.0, 1.0);
    push_v(buf, x0, y1, 0.0, 1.0);
}

// -- Rounded rectangle ---------------------------------------------------

// -- Circle (semi-circular top) ------------------------------------------

#[inline]
fn build_circle_top(
    buf: &mut Vec<f32>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    _rx: f32,
    ry_req: f32,
    segments: u32,
) {
    let w = x1 - x0;
    let h = y1 - y0;
    // Radius in X is always half the bar width; in Y we cap at ry_req.
    let rx = w * 0.5;
    let ry = ry_req.min(h).max(0.0);

    if ry <= 1e-6 {
        build_rectangle(buf, x0, y0, x1, y1);
        return;
    }

    // Bottom rect up to the start of the dome.
    build_rectangle(buf, x0, y0, x1, y1 - ry);

    // Dome fan centered at (cx, y1 - ry), sweeping 0..pi
    let cx = x0 + rx;
    let cy = y1 - ry;
    let segs = segments.max(8);
    let pi = std::f32::consts::PI;

    for i in 0..segs {
        let a0 = (i as f32 / segs as f32) * pi;
        let a1 = ((i + 1) as f32 / segs as f32) * pi;
        // Invert the y direction: angle 0 -> right edge (a1 top), so that the
        // semicircle rises from the rectangle top.
        let (p0x, p0y) = (cx + rx * a0.cos(), cy + ry * a0.sin());
        let (p1x, p1y) = (cx + rx * a1.cos(), cy + ry * a1.sin());
        let uv_c = uv_for(cx, cy, x0, y0, w, h);
        let uv_0 = uv_for(p0x, p0y, x0, y0, w, h);
        let uv_1 = uv_for(p1x, p1y, x0, y0, w, h);
        push_v(buf, cx, cy, uv_c.0, uv_c.1);
        push_v(buf, p0x, p0y, uv_0.0, uv_0.1);
        push_v(buf, p1x, p1y, uv_1.0, uv_1.1);
    }
}

// -- Triangle-top bar ----------------------------------------------------

#[inline]
fn build_triangle_top(buf: &mut Vec<f32>, x0: f32, y0: f32, x1: f32, y1: f32) {
    let w = x1 - x0;
    let h = y1 - y0;
    // Triangle roof takes the top 15% of the bar (visually nicer than full tip).
    let tri_h = h * 0.2;
    let body_top = y1 - tri_h;

    build_rectangle(buf, x0, y0, x1, body_top);

    let cx = x0 + w * 0.5;
    // Triangle tip (single triangle)
    let uv_l = uv_for(x0, body_top, x0, y0, w, h);
    let uv_r = uv_for(x1, body_top, x0, y0, w, h);
    let uv_t = uv_for(cx, y1, x0, y0, w, h);
    push_v(buf, x0, body_top, uv_l.0, uv_l.1);
    push_v(buf, x1, body_top, uv_r.0, uv_r.1);
    push_v(buf, cx, y1, uv_t.0, uv_t.1);
}

#[inline]
fn build_line_bar(buf: &mut Vec<f32>, x0: f32, y0: f32, x1: f32, y1: f32) {
    let thickness = ((x1 - x0).abs() * 0.15).max(0.002);
    let cx = (x0 + x1) * 0.5;
    build_rectangle(buf, cx - thickness, y0, cx + thickness, y1);
}

// -- Helpers -------------------------------------------------------------

#[inline(always)]
fn uv_for(x: f32, y: f32, x0: f32, y0: f32, w: f32, h: f32) -> (f32, f32) {
    let u = if w > 1e-9 { (x - x0) / w } else { 0.0 };
    let v = if h > 1e-9 { (y - y0) / h } else { 0.0 };
    (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}
