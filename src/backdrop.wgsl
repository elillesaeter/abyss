// Backdrop: the sky and the parallax layers, painted first so the world draws over
// it. Each layer is a strip of (top, bottom) silhouette heights generated on the
// CPU; this pass only works out which layer covers a pixel and in which shade.

struct Layer {
    rim: vec4<f32>,
    body: vec4<f32>,
    low: vec4<f32>,
    factor: f32,
    baseline: f32,
    drift: f32,
    pad: f32,
};

struct Backdrop {
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    origin: vec2<f32>,
    camera: vec2<f32>,
    ppt: f32,
    horizon: f32,
    time: f32,
    stars: f32,
    light: vec4<f32>,
    sun: vec4<f32>,
    sun_colour: vec4<f32>,
    moon: vec4<f32>,
    layers: array<Layer, 5>,
};

@group(0) @binding(0) var<uniform> backdrop: Backdrop;
@group(0) @binding(1) var strips: texture_2d<f32>;

const ART_PX: f32 = 16.0;
/// Lit top edge, in art pixels.
const RIM_PX: f32 = 2.0;
/// Shadow under a cloud, in art pixels. Ground layers have no bottom to shade.
const SHADE_PX: f32 = 3.0;
/// The sky is a few hard bands rather than a smooth ramp, like painted pixel art.
const SKY_BANDS: f32 = 8.0;
/// Where the gradient starts (tiles above the screen centre) and how far it runs.
const SKY_FLOOR: f32 = -10.0;
const SKY_SPAN: f32 = 48.0;
/// Stars per art pixel of sky, as one in this many.
const STAR_RARITY: u32 = 1400u;
/// Stars drift this much with the camera: almost pinned, just enough to feel far.
const STAR_FACTOR: f32 = 0.02;
const MOON_LIT: vec3<f32> = vec3<f32>(0.85, 0.87, 0.80);

fn hash2(x: i32, y: i32) -> u32 {
    var h = u32(x) * 747796405u + u32(y) * 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the screen.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_backdrop(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let world = backdrop.origin + at.xy / backdrop.ppt;
    let above = backdrop.camera.y - world.y;
    let t = clamp((above - SKY_FLOOR) / SKY_SPAN, 0.0, 0.999);
    var colour = mix(backdrop.sky_horizon, backdrop.sky_top, floor(t * SKY_BANDS) / (SKY_BANDS - 1.0));

    // Stars: single art pixels, a few brighter, each twinkling at its own rate.
    let star_follow = 1.0 - STAR_FACTOR;
    let star = vec2<i32>(floor(vec2<f32>(
        world.x - backdrop.camera.x * star_follow,
        world.y - (backdrop.camera.y - backdrop.horizon) * star_follow,
    ) * ART_PX));
    let h = hash2(star.x, star.y);
    if backdrop.stars > 0.0 && h % STAR_RARITY == 0u {
        let bright = 0.35 + 0.65 * f32((h >> 12u) & 255u) / 255.0;
        let twinkle = 0.75 + 0.25 * sin(backdrop.time * (1.5 + f32((h >> 20u) & 7u)) + f32(h & 63u));
        colour += vec4<f32>(vec3<f32>(bright * twinkle * backdrop.stars), 0.0);
    }

    // Sun and moon, on the art-pixel grid, behind every layer so a low sun sinks
    // behind the mountains.
    let at_px = floor(world * ART_PX);
    if backdrop.sun.z > 0.0 {
        let d = length(at_px - floor(backdrop.sun.xy * ART_PX));
        let r = backdrop.sun.z;
        if d <= r {
            colour = backdrop.sun_colour;
        } else if d <= r * 1.6 {
            colour = mix(colour, backdrop.sun_colour, 0.35);
        } else if d <= r * 2.6 {
            colour = mix(colour, backdrop.sun_colour, 0.15);
        }
    }
    if backdrop.moon.z > 0.0 {
        let offset = (at_px - floor(backdrop.moon.xy * ART_PX)) / backdrop.moon.z;
        if length(offset) <= 1.0 {
            // Lit where the terminator allows: from the right while waxing, the
            // left while waning, all of it at full (phase 0.5).
            let phase = backdrop.moon.w * 6.2831853;
            let edge = sqrt(max(1.0 - offset.y * offset.y, 0.0));
            var lit = false;
            if phase <= 3.1415927 {
                lit = offset.x > cos(phase) * edge;
            } else {
                lit = offset.x < -cos(phase) * edge;
            }
            // The dark side still hides the stars behind it.
            colour = select(mix(colour, vec4<f32>(MOON_LIT, 1.0), 0.06), vec4<f32>(MOON_LIT, 1.0), lit);
        } else if length(offset) <= 1.8 {
            // A halo, as bright as the moon is full.
            let fullness = (1.0 - cos(backdrop.moon.w * 6.2831853)) / 2.0;
            colour = mix(colour, vec4<f32>(MOON_LIT, 1.0), 0.08 * fullness);
        }
    }

    let width = i32(textureDimensions(strips).x);
    for (var i = 0; i < 5; i++) {
        let layer = backdrop.layers[i];
        // A layer follows `factor` of the camera's movement. Vertically it is
        // anchored to the horizon: with the camera there, every layer lines up.
        let follow = 1.0 - layer.factor;
        let x = world.x - backdrop.camera.x * follow + layer.drift * backdrop.time;
        let height = backdrop.horizon - world.y + (backdrop.camera.y - backdrop.horizon) * follow;
        // Whole art pixels in the layer's own space, so it keeps the pixel grid.
        let column = ((i32(floor(x * ART_PX)) % width) + width) % width;
        let r = floor(height * ART_PX) - layer.baseline;
        let span = textureLoad(strips, vec2<i32>(column, i), 0).rg;
        if r < span.y || r >= span.x {
            continue;
        }
        var shade = layer.body;
        if r >= span.x - RIM_PX {
            shade = layer.rim;
        } else if r < span.y + SHADE_PX {
            shade = layer.low;
        }
        colour = vec4<f32>(shade.rgb * backdrop.light.rgb, 1.0);
    }
    return colour;
}
