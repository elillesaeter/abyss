// Backdrop: the sky and the parallax layers, painted first so the world draws over
// it. Each layer is a strip of (top, bottom) silhouette heights generated on the
// CPU; this pass only works out which layer covers a pixel and in which shade.

struct Layer {
    rim: vec4<f32>,
    body: vec4<f32>,
    low: vec4<f32>,
    /// Cloud (1) or ground (0), height in art pixels, foot haze, mist at its foot.
    shape: vec4<f32>,
    factor: f32,
    baseline: f32,
    drift: f32,
    /// How far the lit layer fades toward the sky's horizon: distance.
    haze: f32,
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
    /// Mist, how low the sun is, and the direction toward the light.
    air: vec4<f32>,
    layers: array<Layer, 5>,
};

@group(0) @binding(0) var<uniform> backdrop: Backdrop;
@group(0) @binding(1) var strips: texture_2d<f32>;
/// Cloud vapour, one map of `CLOUD_PX` rows per cloud layer, row 0 at its base.
@group(0) @binding(2) var clouds: texture_2d<f32>;

// The backdrop is in between pixel art and painting: shapes keep the art-pixel
// grid (silhouettes, stars), while light is smooth (sky, glows, haze, mist).

const ART_PX: f32 = 16.0;
/// Lit top edge of a ground layer, in art pixels.
const RIM_PX: f32 = 2.0;
/// Where the gradient starts (tiles above the screen centre) and how far it runs.
const SKY_FLOOR: f32 = -10.0;
const SKY_SPAN: f32 = 48.0;
/// The paler band of haze along the bottom of the sky: its share of the gradient.
const HAZE_BAND: f32 = 0.3;
/// Stars per art pixel of sky, as one in this many.
const STAR_RARITY: u32 = 1400u;
/// Stars drift this much with the camera: almost pinned, just enough to feel far.
const STAR_FACTOR: f32 = 0.02;
const MOON_LIT: vec3<f32> = vec3<f32>(0.85, 0.87, 0.80);
/// The softness of the moon's terminator, as a share of its radius.
const TERMINATOR: f32 = 0.12;
/// Mist: a band around a layer's base reaching this far above it and half as
/// far below, in art pixels, and how thick it gets at most.
const MIST_REACH: f32 = 72.0;
const MIST_MAX: f32 = 0.55;
const CLOUD_PX: i32 = 192;
/// Samples taken toward the light through a cloud, and how far apart, in art
/// pixels: the vapour in the way is what shades it.
const CLOUD_STEPS: i32 = 5;
const CLOUD_STEP: f32 = 5.0;
/// Cloud colour in its own shadow: cooler, lit by the blue sky rather than the sun.
const CLOUD_SHADOW: vec3<f32> = vec3<f32>(0.56, 0.6, 0.74);

fn hash2(x: i32, y: i32) -> u32 {
    var h = u32(x) * 747796405u + u32(y) * 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

fn unit2(p: vec2<i32>) -> f32 {
    return f32(hash2(p.x, p.y) & 0xFFFFu) / 65535.0;
}

/// Smooth value noise in 0..1.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = unit2(i);
    let b = unit2(i + vec2<i32>(1, 0));
    let c = unit2(i + vec2<i32>(0, 1));
    let d = unit2(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// The moon's face at a point on its disc (unit radius): brightness 0..1 from
/// darker seas and a few craters. Fixed, so it is the same moon every night.
fn moon_face(p: vec2<f32>) -> f32 {
    let seas = value_noise(p * 2.2 + vec2<f32>(5.3, 1.7)) * 0.7 + value_noise(p * 5.0 + vec2<f32>(2.1, 9.4)) * 0.3;
    var face = 1.0 - 0.28 * smoothstep(0.48, 0.62, seas);
    let craters = array<vec3<f32>, 5>(
        vec3<f32>(-0.35, 0.42, 0.13),
        vec3<f32>(0.4, -0.3, 0.1),
        vec3<f32>(0.15, 0.55, 0.07),
        vec3<f32>(-0.55, -0.2, 0.08),
        vec3<f32>(0.5, 0.35, 0.06),
    );
    for (var i = 0; i < 5; i++) {
        let c = craters[i];
        let d = length(p - c.xy) / c.z;
        // A shaded bowl with a brighter rim on its lower right, lit from the sun.
        if d < 1.0 {
            face *= 0.84 + 0.1 * d;
        } else if d < 1.35 && dot(p - c.xy, vec2<f32>(0.7, 0.7)) > 0.0 {
            face *= 1.06;
        }
    }
    return face;
}

/// Vapour of cloud map `slot` at art pixel `p` (x along the strip, y above its
/// base), blended between pixels: it wraps across and is empty above and below.
fn vapour(slot: i32, p: vec2<f32>) -> f32 {
    let width = i32(textureDimensions(clouds).x);
    let q = p - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    var v = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    for (var k = 0; k < 4; k++) {
        let c = i + vec2<i32>(k & 1, k >> 1);
        if c.y >= 0 && c.y < CLOUD_PX {
            v[k] = textureLoad(clouds, vec2<i32>(((c.x % width) + width) % width, slot * CLOUD_PX + c.y), 0).r;
        }
    }
    return mix(mix(v[0], v[1], f.x), mix(v[2], v[3], f.x), f.y);
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
    let pixel = 1.0 / backdrop.ppt;
    let above = backdrop.camera.y - world.y;
    let t = clamp((above - SKY_FLOOR) / SKY_SPAN, 0.0, 1.0);
    let mist = backdrop.air.x;
    let low_sun = backdrop.air.y;

    // Sky: a smooth gradient, eased so the blue deepens overhead, with a paler
    // band of haze along the bottom.
    var sky = mix(backdrop.sky_horizon.rgb, backdrop.sky_top.rgb, pow(t, 0.8));
    let haze = min(backdrop.sky_horizon.rgb * 1.12 + vec3<f32>(0.02), vec3<f32>(1.0));
    sky = mix(sky, haze, pow(1.0 - min(t / HAZE_BAND, 1.0), 2.0) * 0.6);

    // The sun lights the sky around it, more widely the lower it is, and at
    // sunrise and sunset warms the whole side of the sky it is on.
    if backdrop.sun.z > 0.0 {
        let sun_r = backdrop.sun.z / ART_PX;
        let d = length(world - backdrop.sun.xy);
        let reach = sun_r * (2.5 + 5.0 * low_sun);
        sky += backdrop.sun_colour.rgb * exp(-max(d - sun_r, 0.0) / reach) * (0.35 + 0.25 * low_sun);
        let side = exp(-abs(world.x - backdrop.sun.x) / 40.0) * (1.0 - t) * low_sun;
        sky = mix(sky, backdrop.sun_colour.rgb * backdrop.light.rgb * 1.2, side * 0.35);
    }
    var colour = sky;

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
        colour += vec3<f32>(bright * twinkle * backdrop.stars);
    }

    // Sun and moon, smooth at screen resolution, behind every layer so a low sun
    // sinks behind the mountains.
    if backdrop.sun.z > 0.0 {
        let d = length(world - backdrop.sun.xy);
        let disc = 1.0 - smoothstep(backdrop.sun.z / ART_PX - pixel, backdrop.sun.z / ART_PX + pixel, d);
        colour = mix(colour, min(backdrop.sun_colour.rgb * 1.15, vec3<f32>(1.0)), disc);
    }
    if backdrop.moon.z > 0.0 {
        let r = backdrop.moon.z / ART_PX;
        let offset = (world - backdrop.moon.xy) / r;
        let d = length(offset);
        // A halo, as bright as the moon is full.
        let fullness = (1.0 - cos(backdrop.moon.w * 6.2831853)) / 2.0;
        colour += MOON_LIT * exp(-max(d - 1.0, 0.0) / 1.2) * 0.1 * fullness;
        let disc = 1.0 - smoothstep(1.0 - pixel / r, 1.0 + pixel / r, d);
        if disc > 0.0 {
            // Lit where the terminator allows: from the right while waxing, the
            // left while waning, all of it at full (phase 0.5). Soft, as a curved
            // surface turns away from the sun.
            let phase = backdrop.moon.w * 6.2831853;
            let edge = sqrt(max(1.0 - offset.y * offset.y, 0.0));
            let across = select(-cos(phase) * edge - offset.x, offset.x - cos(phase) * edge, phase <= 3.1415927);
            let lit = smoothstep(-TERMINATOR, TERMINATOR, across);
            // The dark side is the sky without its stars, faintly earthlit.
            let dark = sky + MOON_LIT * 0.04;
            let face = MOON_LIT * moon_face(offset);
            colour = mix(colour, mix(dark, face, lit), disc);
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
        if layer.shape.x > 0.5 {
            // Cloud: vapour from its map, on the pixel grid unless soft (far
            // haze). Thin vapour lets the sky through; the vapour between a
            // point and the light shades it, so tops facing the light are bright
            // and undersides cool. Faint by moonlight.
            let slot = i32(layer.shape.x) - 1;
            var p = vec2<f32>(x * ART_PX, height * ART_PX - layer.baseline);
            if layer.shape.z < 0.5 {
                p = floor(p) + 0.5;
            }
            let d = vapour(slot, p);
            if d > 0.004 {
                let toward = vec2<f32>(backdrop.air.z, -backdrop.air.w);
                var in_way = 0.0;
                for (var k = 1; k <= CLOUD_STEPS; k++) {
                    in_way += vapour(slot, p + toward * CLOUD_STEP * f32(k));
                }
                let lit = exp(-in_way * 0.35);
                let base = layer.body.rgb * backdrop.light.rgb * mix(1.0, 0.35, backdrop.stars);
                var shade = base * mix(CLOUD_SHADOW, vec3<f32>(1.0), lit);
                shade = mix(shade, backdrop.sky_horizon.rgb, layer.haze);
                // Even a core lets a little sky through: vapour, not paint.
                colour = mix(colour, shade, smoothstep(0.0, 0.9, d) * 0.85);
            }
            continue;
        }
        // Whole art pixels in the layer's own space, so it keeps the pixel grid.
        let column = ((i32(floor(x * ART_PX)) % width) + width) % width;
        let r = floor(height * ART_PX) - layer.baseline;
        let span = textureLoad(strips, vec2<i32>(column, i), 0).rg;
        let tall = layer.shape.y;
        if r >= span.y && r < span.x {
            let shade = select(layer.body.rgb, layer.rim.rgb, r >= span.x - RIM_PX);
            colour = mix(shade * backdrop.light.rgb, backdrop.sky_horizon.rgb, layer.haze);
            // Haze lies thickest near the ground: the foot fades into the sky.
            let foot = pow(1.0 - clamp(r / max(tall, 1.0), 0.0, 1.0), 1.5) * layer.shape.z;
            colour = mix(colour, sky, foot);
        }
        // Morning mist: soft drifting bands lying over this layer's foot and the
        // farther layers behind it, faded out with height.
        if layer.shape.w > 0.5 && mist > 0.0 && abs(r) < MIST_REACH {
            let fall = smoothstep(-MIST_REACH * 0.5, 0.0, r) * (1.0 - smoothstep(0.0, MIST_REACH, r));
            let bands = value_noise(vec2<f32>(x * 0.25 + backdrop.time * 0.05, r / 18.0)) * 0.6
                + value_noise(vec2<f32>(x * 0.9 - backdrop.time * 0.08, r / 7.0)) * 0.4;
            let density = mist * MIST_MAX * fall * smoothstep(0.25, 0.75, bands + 0.25 * fall);
            // Pale: mostly white, taking a little of the sky's colour.
            let veil = mix(haze, vec3<f32>(0.85, 0.86, 0.88), 0.5) * backdrop.light.rgb + vec3<f32>(0.04);
            colour = mix(colour, veil, density);
        }
    }

    // Dither by under one display step, so the smooth sky doesn't band.
    let n = (f32(hash2(i32(at.x), i32(at.y)) & 0xFFu) / 255.0 - 0.5) / 255.0;
    let srgb = pow(max(colour, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)) + vec3<f32>(n);
    return vec4<f32>(pow(max(srgb, vec3<f32>(0.0)), vec3<f32>(2.2)), 1.0);
}
