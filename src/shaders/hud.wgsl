// HUD overlay: the CPU-drawn image (premultiplied alpha), pixel for pixel over the frame.

@group(0) @binding(0) var hud: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(hud));
    let xy = min(vec2<i32>(p.xy), size - 1);
    return textureLoad(hud, xy, 0);
}
