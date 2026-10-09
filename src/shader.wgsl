struct View {
    scale: vec2<f32>,
    offset: vec2<f32>,
    atlas_size: vec2<f32>,
    cell: f32,
    mask_cols: f32,
    mask_band_y: f32,
    block: f32,
    block_tiles: f32,
    pad: f32,
    light_origin: vec2<f32>,
    light_size: vec2<f32>,
    direct: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var light_map: texture_2d<f32>;
@group(0) @binding(4) var light_sampler: sampler;
@group(0) @binding(5) var sun_map: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) mat_uv: vec2<f32>,
    @location(1) mask_uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) light_uv: vec2<f32>,
    @location(4) @interpolate(flat) id: u32,
};

/// Colour, plus which object covers the pixel for the outline pass to compare.
struct FsOut {
    @location(0) colour: vec4<f32>,
    @location(1) id: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) instance_pos: vec2<f32>,
    @location(1) instance_axis_x: vec2<f32>,
    @location(2) instance_axis_y: vec2<f32>,
    @location(3) instance_cells: vec2<u32>,
    @location(4) instance_tint: vec4<f32>,
    @location(5) instance_id: u32,
) -> VsOut {
    // Two triangles forming the unit quad; no vertex buffer needed.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];
    let tile = instance_pos + corner.x * instance_axis_x + corner.y * instance_axis_y;

    let mask = f32(instance_cells.x);
    // Low byte: material column. High byte: which cell of its block, picked on the
    // CPU from the tile the quad belongs to.
    let mat = f32(instance_cells.y & 0xFFu);
    let sub = vec2<f32>(f32((instance_cells.y >> 8u) & 0xFu), f32((instance_cells.y >> 12u) & 0xFu));

    // Masks are one tile each, `mask_cols` per row, below the material band.
    let mask_origin = vec2<f32>(
        (mask % view.mask_cols) * view.cell,
        view.mask_band_y + floor(mask / view.mask_cols) * view.cell,
    );

    // Material blocks span `block_tiles` tiles; sampling the sub-cell matching the
    // tile's world position runs the pattern continuously across tile boundaries
    // instead of restarting in every tile.
    let mat_origin = vec2<f32>(mat * view.block, 0.0) + sub * view.cell;

    var out: VsOut;
    out.clip = vec4<f32>(tile * view.scale + view.offset, 0.0, 1.0);
    out.mat_uv = (mat_origin + corner * view.cell) / view.atlas_size;
    out.mask_uv = (mask_origin + corner * view.cell) / view.atlas_size;
    out.tint = instance_tint;
    // Light is a tile-resolution texture over the visible region; sampling it by
    // world position with linear filtering gives a gradient across and between
    // tiles, and lights sprites through the same lookup.
    out.light_uv = (tile - view.light_origin) / view.light_size;
    out.id = instance_id;
    return out;
}

fn shade(in: VsOut) -> vec4<f32> {
    let material = textureSample(atlas, atlas_sampler, in.mat_uv);
    // The mask carries shape in its alpha only; the colour comes from the material.
    let coverage = textureSample(atlas, atlas_sampler, in.mask_uv).a;
    // Spread light (sky, bounce, torches) plus direct sun or moonlight, which has
    // its own finer texture over the same region so its shadows stay sharp.
    let spread = textureSample(light_map, light_sampler, in.light_uv).rgb;
    let direct = view.direct.rgb * textureSample(sun_map, light_sampler, in.light_uv).r;
    let light = min(spread + direct, vec3<f32>(1.0));
    let rgba = material * in.tint;
    return vec4<f32>(rgba.rgb * light, rgba.a * coverage);
}

/// Objects and terrain bases: a pixel either belongs to the object or it doesn't,
/// because an id cannot be half written.
@fragment
fn fs_main(in: VsOut) -> FsOut {
    let colour = shade(in);
    if colour.a < 0.5 {
        discard;
    }
    return FsOut(colour, in.id);
}

/// Soft transition overlays: blended, and drawn with id writes masked off, since the
/// terrain base under them has already recorded whose pixel it is.
@fragment
fn fs_soft(in: VsOut) -> FsOut {
    return FsOut(shade(in), in.id);
}
