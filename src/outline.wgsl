// Final pass: copies the scene to the screen, darkening any pixel that has
// something *behind* it within `width` pixels. Ids grow with draw order, so
// "behind" is "smaller id", and the background (0) is behind everything.
// Open air also glows where direct light passes through it: the god rays.

struct Outline {
    colour: vec4<f32>,
    ray: vec4<f32>,
    origin: vec2<f32>,
    source: vec2<f32>,
    ppt: f32,
    width: i32,
    pad: vec2<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var ids: texture_2d<u32>;
@group(0) @binding(2) var<uniform> outline: Outline;

/// Samples taken along each beam, and how far toward the sun they look, in tiles.
const RAY_SAMPLES: i32 = 40;
const RAY_REACH: f32 = 30.0;
/// The open share of a line is raised to this power: each blocked stretch dims
/// the beam multiplicatively, as light through layer after layer of leaves does,
/// rather than an average that blurs a canopy's gaps away.
const RAY_SHARPNESS: f32 = 6.0;

/// Whether nothing is drawn in front of the backdrop at a screen point: leaves,
/// wood and ground block a beam; off screen counts as open. The parallel shadow
/// texture must not count here — its straight shadows would smear across the fan.
fn open(px: vec2<f32>) -> f32 {
    let q = vec2<i32>(floor(px));
    let size = vec2<i32>(textureDimensions(ids));
    if all(q >= vec2<i32>(0)) && all(q < size) && textureLoad(ids, q, 0).r != 0u {
        return 0.0;
    }
    return 1.0;
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the screen.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_outline(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(at.xy);
    let size = vec2<i32>(textureDimensions(ids));
    let mine = textureLoad(ids, p, 0).r;
    let w = outline.width;
    if mine != 0u {
        for (var dy = -w; dy <= w; dy++) {
            for (var dx = -w; dx <= w; dx++) {
                // A disc, so corners are rounded rather than squared off.
                if dx * dx + dy * dy > w * w {
                    continue;
                }
                let q = p + vec2<i32>(dx, dy);
                if any(q < vec2<i32>(0)) || any(q >= size) {
                    continue;
                }
                if textureLoad(ids, q, 0).r < mine {
                    return outline.colour;
                }
            }
        }
        return textureLoad(scene, p, 0);
    }
    // Background: air, with the backdrop behind it. Air in the light scatters a
    // little of it toward you. Each pixel looks back along the line to the sun
    // *on screen* and glows by how much of that line is open, so beams fan out
    // from the sun as perspective makes real ones appear to, while the light on
    // the ground stays parallel and steady. A per-pixel offset to where sampling
    // starts trades banding for fine noise.
    let source = (outline.source - outline.origin) * outline.ppt;
    let to = source - at.xy;
    let reach = min(length(to), RAY_REACH * outline.ppt);
    let step = normalize(to + vec2<f32>(0.0, -1e-4)) * reach / f32(RAY_SAMPLES);
    let jitter = fract(52.9829189 * fract(dot(at.xy, vec2<f32>(0.06711056, 0.00583715))));
    var sum = 0.0;
    for (var i = 0; i < RAY_SAMPLES; i++) {
        sum += open(at.xy + step * (f32(i) + jitter));
    }
    // Underground the line to the sun runs through rock, so caves get no beams.
    let lit = pow(sum / f32(RAY_SAMPLES), RAY_SHARPNESS);
    let behind = textureLoad(scene, p, 0);
    return vec4<f32>(behind.rgb + outline.ray.rgb * lit, behind.a);
}
