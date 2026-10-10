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
    /// Where the pixel is in the world, in tiles, and the tile the quad covers, in
    /// art pixels, for biting ragged edges.
    @location(5) world: vec2<f32>,
    @location(6) @interpolate(flat) tile_px: vec2<f32>,
    @location(7) @interpolate(flat) mask_origin: vec2<f32>,
    @location(8) @interpolate(flat) edges: u32,
};

/// Sides a tile can face air on, in `edges`' low bits, matching autotile's order.
const EDGE_N: u32 = 1u;
const EDGE_E: u32 = 2u;
const EDGE_S: u32 = 4u;
const EDGE_W: u32 = 8u;
const EDGE_BLOCKY: u32 = 16u;
/// Stone breaks in runs this many art pixels wide.
const BLOCK_RUN: f32 = 3.0;

fn hash2(x: i32, y: i32) -> u32 {
    var h = u32(x) * 747796405u + u32(y) * 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

/// How many art pixels to bite off an open side at one point along it. Keyed by
/// the world, not the tile: `along` is the art-pixel column (or row) and `line`
/// the boundary it lies on, so a ragged line runs on across tiles and never
/// repeats. Dirt crumbles pixel by pixel; stone breaks in blocky runs, deeper.
fn bite(along: f32, line: f32, side: u32, blocky: bool) -> f32 {
    let key = i32(line) * 8 + i32(side);
    if blocky {
        // Runs shifted per line, so they don't stack into a grid.
        let shift = f32(hash2(key, 7) % 3u);
        let v = hash2(i32(floor((along + shift) / BLOCK_RUN)), key) % 20u;
        return select(select(0.0, 1.0, v >= 8u), 2.0, v >= 15u);
    }
    // Dirt: lumps one or two pixels wide. Each pair of pixels either splits or
    // moves as one, so the lumps keep no steady rhythm.
    let pair = i32(floor(along / 2.0));
    let lump = select(pair * 2, i32(along), (hash2(pair, key) & 1u) == 1u);
    let v = hash2(lump, key + 5) % 16u;
    return select(select(0.0, 1.0, v >= 10u), 2.0, v >= 15u);
}

/// Whether a pixel survives a tile's ragged edges, and how far to move its mask
/// lookup so rims and grass sit along the bitten edge instead of being cut off.
struct Ragged {
    keep: f32,
    shift: vec2<f32>,
};

fn ragged(in: VsOut) -> Ragged {
    var out = Ragged(1.0, vec2<f32>(0.0));
    if in.edges == 0u {
        return out;
    }
    let px = floor(in.world * view.cell);
    let local = px - in.tile_px;
    let last = view.cell - 1.0;
    let size = view.cell;
    let blocky = (in.edges & EDGE_BLOCKY) != 0u;
    var b = vec4<f32>(0.0); // N, E, S, W
    if (in.edges & EDGE_N) != 0u { b.x = bite(px.x, in.tile_px.y / size, 0u, blocky); }
    if (in.edges & EDGE_E) != 0u { b.y = bite(px.y, in.tile_px.x / size + 1.0, 1u, blocky); }
    if (in.edges & EDGE_S) != 0u { b.z = bite(px.x, in.tile_px.y / size + 1.0, 2u, blocky); }
    if (in.edges & EDGE_W) != 0u { b.w = bite(px.y, in.tile_px.x / size, 3u, blocky); }
    if local.y < b.x || last - local.y < b.z || local.x < b.w || last - local.x < b.y {
        out.keep = 0.0;
    }

    // Outer corners, where two open sides meet, lose a nick: a diagonal step on
    // dirt, a square notch on stone.
    let inset = vec2<f32>(select(last - local.x, local.x, local.x < size * 0.5), select(last - local.y, local.y, local.y < size * 0.5));
    let h = in.edges & 15u;
    let corner = select(
        select((h & (EDGE_S | EDGE_E)) == (EDGE_S | EDGE_E), (h & (EDGE_N | EDGE_E)) == (EDGE_N | EDGE_E), local.y < size * 0.5),
        select((h & (EDGE_S | EDGE_W)) == (EDGE_S | EDGE_W), (h & (EDGE_N | EDGE_W)) == (EDGE_N | EDGE_W), local.y < size * 0.5),
        local.x < size * 0.5,
    );
    if corner {
        let nick = 1.0 + f32(hash2(i32(in.tile_px.x), i32(in.tile_px.y)) & 1u);
        if (blocky && max(inset.x, inset.y) < 2.0) || (!blocky && inset.x + inset.y < nick) {
            out.keep = 0.0;
        }
    }

    out.shift = vec2<f32>(select(b.y, -b.w, local.x < size * 0.5), select(b.z, -b.x, local.y < size * 0.5));
    return out;
}

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
    @location(6) instance_edges: u32,
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
    out.world = tile;
    out.tile_px = instance_pos * view.cell;
    out.mask_origin = mask_origin;
    out.edges = instance_edges;
    return out;
}

fn shade(in: VsOut) -> vec4<f32> {
    let material = textureSample(atlas, atlas_sampler, in.mat_uv);
    // The mask carries shape in its alpha only; the colour comes from the material.
    // A ragged tile looks its mask up per art pixel, shifted along with the bite.
    let edge = ragged(in);
    let local = floor(in.world * view.cell) - in.tile_px;
    let shifted = (in.mask_origin + clamp(local + edge.shift, vec2<f32>(0.0), vec2<f32>(view.cell - 1.0)) + 0.5) / view.atlas_size;
    let mask_uv = select(in.mask_uv, shifted, in.edges != 0u);
    let coverage = textureSample(atlas, atlas_sampler, mask_uv).a * edge.keep;
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
