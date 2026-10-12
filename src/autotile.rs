//! Turns tiles into draw instances: one opaque base per tile, plus one overlay
//! per material that bleeds into it from a higher precedence.
//!
//! An overlay samples the bleeding material's texture through a mask chosen by
//! the 8-neighbour bitmask of *that* material, so a single quad covers edges,
//! outer corners and diagonal-only nubs together. Emitting separate per-edge and
//! per-corner quads would double-blend where they overlap and leave dark seams.
//!
//! Decoration (grass today) uses the same mechanism against the tile's air-facing
//! sides, so grass is a skin on dirt rather than a block of its own.

use crate::atlas::{self, DIRS, SOLID_MASK};
use crate::tiles::{AIR, DECOR_TYPES, STONE, TileId, atlas_mat, drawn_as, is_solid, precedence};
use crate::atlas::{MAT_AIR_SHADOW, MAT_RIM, MAT_SKY, THIN_MASKS};
use crate::world::World;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    /// Corner the quad's edges start from, in tile units (top-left when unrotated).
    pub pos: [f32; 2],
    /// The quad's two edges as vectors: `[w, 0]` and `[0, h]` for a plain box.
    /// Edges rather than a size so a quad can lean (sway) or rotate (a falling tree).
    pub axis_x: [f32; 2],
    pub axis_y: [f32; 2],
    /// Mask cell index; `SOLID_MASK` for fully opaque quads.
    pub mask: u16,
    /// Material column in the low byte, and which cell of its block to sample in
    /// the high byte (see `textured`).
    pub mat: u16,
    pub tint: [u8; 4],
    /// Which object this quad belongs to, for outlining. Grows with draw order, so
    /// a smaller id is behind: an outline is drawn where something meets what is
    /// behind it, and never inside one object.
    pub id: u32,
    /// Which sides of the tile face air (N, E, S, W in the low bits) plus
    /// `EDGE_BLOCKY`: the shader bites a ragged line into those sides. Zero for
    /// anything that keeps its quad's straight edges.
    pub edges: u32,
}

/// Sky, and anything else the world shows through; never outlined itself.
pub const ID_BACKGROUND: u32 = 0;
/// All terrain is one object, so blended material boundaries carry no outline.
pub const ID_TERRAIN: u32 = 1;
/// Trees, mobs and the player take ids from here up, in the order they are drawn.
pub const FIRST_OBJECT_ID: u32 = 2;
/// The edge breaks in short blocky runs, as stone does, rather than crumbling.
pub const EDGE_BLOCKY: u32 = 1 << 4;
/// Not an edge: a round piece of wood, lit a little more on the side facing the
/// light. Its first axis runs across it.
pub const LIT_SIDE: u32 = 1 << 5;
/// The four orthogonal neighbour bits: only those sides get bitten.
const SIDES: u8 = 0b1111;

/// One chunk's instances, in the order they are drawn.
#[derive(Default)]
pub struct Layers {
    /// The underground darkness behind holes. Drawn under the terrain, so it shows
    /// where a ragged edge bites into a tile as well as through open air.
    pub veils: Vec<Instance>,
    pub bases: Vec<Instance>,
    pub overlays: Vec<Instance>,
}

impl Layers {
    fn clear(&mut self) {
        self.veils.clear();
        self.bases.clear();
        self.overlays.clear();
    }
}

/// A black quad over one tile, as opaque as the tile is far from the sky.
fn veil(world: &World, x: i32, y: i32) -> Option<Instance> {
    let alpha = 255 - (world.skyness(x, y) * 255.0) as u8;
    (alpha > 0).then(|| Instance { tint: [0, 0, 0, alpha], id: ID_BACKGROUND, ..Instance::tile(x, y, SOLID_MASK, MAT_SKY) })
}

impl Instance {
    pub fn tile(x: i32, y: i32, mask: u16, mat: u16) -> Instance {
        Instance {
            pos: [x as f32, y as f32],
            axis_x: [1.0, 0.0],
            axis_y: [0.0, 1.0],
            mask,
            mat: textured(mat, [x, y]),
            tint: [255, 255, 255, 255],
            edges: 0,
            id: ID_TERRAIN,
        }
    }

    /// The material column, without the packed block cell.
    #[cfg(test)]
    pub fn material(&self) -> u16 {
        self.mat & 0xFF
    }
}

/// Packs a material with the cell of its multi-tile block that `anchor` falls on.
///
/// Chosen on the CPU from a tile position the quad belongs to, rather than in the
/// shader from wherever the quad is drawn: a toppling trunk keeps its bark instead
/// of the texture flickering as its corners cross tile boundaries.
pub fn textured(mat: u16, anchor: [i32; 2]) -> u16 {
    let n = atlas::BLOCK_TILES as i32;
    let (sx, sy) = (anchor[0].rem_euclid(n) as u16, anchor[1].rem_euclid(n) as u16);
    mat | (sx << 8) | (sy << 12)
}

/// At most four materials can bleed into one tile (air plus three others), so the
/// per-tile overlay list never allocates.
const MAX_OVERLAYS: usize = 4;
/// Bleeding materials plus any decoration the tile carries.
const MAX_LAYERS: usize = MAX_OVERLAYS + 4;
/// Neighbour bits that face up: north and its two diagonals.
const TOP: u8 = 1 | 1 << 4 | 1 << 7;

/// Appends a tile's instances to `out`: a solid tile's base and overlays, and the
/// veil behind open air or a ragged edge.
pub fn emit_tile(world: &World, x: i32, y: i32, out: &mut Layers) {
    let id = drawn_as(world.get(x, y));
    if id == AIR {
        // A hole shows what is behind it: the backdrop outdoors, nothing at all
        // underground. So air draws a black veil over the backdrop, thickening with
        // depth, which is what makes a dug tunnel read as a tunnel rather than as a
        // window. A gradient rather than a switch: keying on "is anything above me"
        // put a hard seam between a dug shaft showing sky and untouched rock beside
        // it. Blended like the overlays, and leaving the id as background.
        out.veils.extend(veil(world, x, y));
        return;
    }
    if !is_solid(id) {
        // Torches and the like: drawn whole, and taking no transitions.
        out.bases.push(Instance { id: ID_TERRAIN, ..Instance::tile(x, y, SOLID_MASK, atlas_mat(id)) });
        return;
    }

    // Collect the neighbour bitmask per bleeding material, and separately which
    // sides face air, since that is where decoration grows.
    let mut found: [(TileId, u8); MAX_OVERLAYS] = [(AIR, 0); MAX_OVERLAYS];
    let mut count = 0;
    let mut air_bits: u8 = 0;
    for (bit, (dx, dy)) in DIRS.iter().enumerate() {
        let neighbour = drawn_as(world.get(x + dx, y + dy));
        if neighbour == AIR {
            air_bits |= 1 << bit;
        }
        if precedence(neighbour) <= precedence(id) {
            continue;
        }
        match found[..count].iter().position(|(n, _)| *n == neighbour) {
            Some(i) => found[i].1 |= 1 << bit,
            None if count < MAX_OVERLAYS => {
                found[count] = (neighbour, 1 << bit);
                count += 1;
            }
            None => {}
        }
    }

    // Sides facing air get a ragged edge, on the base and on every overlay of the
    // tile, so rims and grass follow the bite. Whatever the bite uncovers must look
    // like the hole beside it, so the tile carries a veil of its own beneath it.
    let sides = air_bits & SIDES;
    let edges = if sides == 0 { 0 } else { sides as u32 | if id == STONE { EDGE_BLOCKY } else { 0 } };
    if edges != 0 {
        out.veils.extend(veil(world, x, y));
    }
    out.bases.push(Instance { edges, ..Instance::tile(x, y, SOLID_MASK, atlas_mat(id)) });

    // (precedence, mask bits, material), painted weakest first.
    let mut layers: [(u8, u16, u16); MAX_LAYERS] = [(0, 0, 0); MAX_LAYERS];
    let mut layer_count = 0;
    let mut push = |layer: (u8, u16, u16)| {
        if layer_count < MAX_LAYERS {
            layers[layer_count] = layer;
            layer_count += 1;
        }
    };
    for &(neighbour, bits) in &found[..count] {
        if neighbour == AIR {
            // A face to air is lit from above: a warm rim where it faces up, a
            // darker lip on the sides and underneath.
            let (top, rest) = (bits & TOP, bits & !TOP);
            if rest != 0 {
                push((precedence(AIR), THIN_MASKS + rest as u16, MAT_AIR_SHADOW));
            }
            if top != 0 {
                push((precedence(AIR), THIN_MASKS + top as u16, MAT_RIM));
            }
        } else {
            push((precedence(neighbour), bits as u16, atlas_mat(neighbour)));
        }
    }
    let decor = world.decor(x, y);
    if decor != 0 && air_bits != 0 {
        for kind in DECOR_TYPES {
            if decor & kind.bit != 0 {
                push((kind.precedence, air_bits as u16, kind.atlas_mat));
            }
        }
    }

    layers[..layer_count].sort_by_key(|(p, _, _)| *p);
    for &(_, bits, mat) in &layers[..layer_count] {
        out.overlays.push(Instance { edges, ..Instance::tile(x, y, bits, mat) });
    }
}

/// Rebuilds one chunk's instance lists. Tiles on the chunk border read neighbours
/// from adjacent chunks, which is why `World::set` bumps their revisions too.
pub fn build_chunk(world: &World, cx: i32, cy: i32, out: &mut Layers) {
    out.clear();
    let size = crate::world::CHUNK_SIZE;
    for y in cy * size..(cy + 1) * size {
        for x in cx * size..(cx + 1) * size {
            emit_tile(world, x, y, out);
        }
    }
}


/// The player and other sprites, until they have atlas cells of their own.
pub fn tinted_box(pos: [f32; 2], size: [f32; 2], tint: [u8; 4], id: u32) -> Instance {
    Instance {
        pos,
        axis_x: [size[0], 0.0],
        axis_y: [0.0, size[1]],
        mask: SOLID_MASK,
        mat: atlas::MAT_WHITE,
        tint,
        edges: 0,
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{DECOR_GRASS, DIRT};

    const N: u16 = 1;
    const E: u16 = 1 << 1;
    const S: u16 = 1 << 2;
    const W: u16 = 1 << 3;
    const NE: u16 = 1 << 4;

    /// 64x64 of stone with a 3-tile margin, so (32, 32) is deep interior.
    fn solid_world() -> World {
        let mut w = World::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                w.set(x, y, STONE);
            }
        }
        w
    }

    fn emitted(world: &World, x: i32, y: i32) -> (Vec<Instance>, Vec<Instance>) {
        let layers = layers(world, x, y);
        (layers.bases, layers.overlays)
    }

    fn layers(world: &World, x: i32, y: i32) -> Layers {
        let mut out = Layers::default();
        emit_tile(world, x, y, &mut out);
        out
    }

    #[test]
    fn an_interior_tile_emits_only_an_opaque_base() {
        let world = solid_world();
        let (bases, overlays) = emitted(&world, 32, 32);
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].mask, SOLID_MASK);
        assert_eq!(bases[0].id, ID_TERRAIN, "every material is one terrain object: no line where they blend");
        assert_eq!(bases[0].material(), atlas_mat(STONE));
        assert!(overlays.is_empty(), "an enclosed tile needs no overlay");
    }

    #[test]
    fn air_above_bleeds_down_as_a_north_overlay() {
        let mut world = solid_world();
        world.set(32, 31, AIR);
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].material(), MAT_RIM, "a face to the sky above gets the lit rim");
        assert_eq!(overlays[0].mask, THIN_MASKS + N, "expected only the north bit");
    }

    #[test]
    fn a_diagonal_only_neighbour_still_emits_its_corner() {
        let mut world = solid_world();
        world.set(33, 31, AIR); // north-east only; orthogonals stay solid
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 1, "a diagonal neighbour must not be dropped");
        assert_eq!(overlays[0].mask, THIN_MASKS + NE);
    }

    #[test]
    fn several_exposed_sides_combine_into_one_overlay() {
        let mut world = solid_world();
        for (x, y) in [(32, 31), (33, 32), (32, 33), (31, 32), (33, 31)] {
            world.set(x, y, AIR);
        }
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 2, "one lip quad and one rim quad, however many sides");
        assert_eq!((overlays[0].material(), overlays[0].mask), (MAT_AIR_SHADOW, THIN_MASKS + (E | S | W)));
        assert_eq!((overlays[1].material(), overlays[1].mask), (MAT_RIM, THIN_MASKS + (N | NE)), "the rim paints over the lip");
    }

    #[test]
    fn precedence_decides_which_way_a_boundary_bleeds() {
        let mut world = solid_world();
        world.set(32, 31, DIRT);

        let (_, onto_stone) = emitted(&world, 32, 32);
        assert_eq!(onto_stone.len(), 1, "dirt outranks stone and should bleed down");
        assert_eq!(onto_stone[0].material(), atlas_mat(DIRT));

        let (_, onto_dirt) = emitted(&world, 32, 31);
        let from_stone = onto_dirt.iter().any(|o| o.material() == atlas_mat(STONE));
        assert!(!from_stone, "stone must not bleed up onto dirt");
    }

    #[test]
    fn grass_grows_on_the_air_facing_sides_of_marked_dirt() {
        let mut world = solid_world();
        world.set(32, 32, DIRT);
        world.set(32, 31, AIR);
        world.set_decor(32, 32, DECOR_GRASS);

        let (_, overlays) = emitted(&world, 32, 32);
        let grass = DECOR_TYPES[0].atlas_mat;
        assert_eq!(overlays.len(), 2, "grass and the air edge both apply");
        assert_eq!(overlays[0].material(), grass, "grass paints before the air edge");
        assert_eq!(overlays[0].mask, N, "grass follows the air-facing sides");
        assert_eq!(overlays[1].material(), MAT_RIM, "the lit rim paints last, over the grass");
    }

    #[test]
    fn buried_dirt_shows_no_grass_even_when_marked() {
        let mut world = solid_world();
        world.set(32, 32, DIRT);
        world.set_decor(32, 32, DECOR_GRASS);
        let (_, overlays) = emitted(&world, 32, 32);
        assert!(overlays.is_empty(), "grass needs a face touching air");
    }

    #[test]
    fn freshly_exposed_dirt_is_bare() {
        let mut world = solid_world();
        for y in 30..34 {
            for x in 30..34 {
                world.set(x, y, DIRT);
            }
        }
        world.set_decor(32, 31, DECOR_GRASS);
        world.set(32, 31, AIR); // mine the grassy tile away

        let (_, overlays) = emitted(&world, 32, 32);
        let grass = DECOR_TYPES[0].atlas_mat;
        assert!(
            !overlays.iter().any(|o| o.material() == grass),
            "dirt uncovered by mining must not sprout grass on its own"
        );
    }

    #[test]
    fn air_shows_the_backdrop_and_veils_it_with_depth() {
        let world = World::new(64, 64);
        let open = layers(&world, 32, 32);
        assert!(open.bases.is_empty(), "open air draws nothing over the backdrop");
        assert!(open.overlays.is_empty(), "and takes no transitions");

        // The veil thickens with depth: a shallow hole still shows the backdrop, a
        // deep one shows nothing. Both matter — veiling immediately makes a dug
        // notch at the surface look like it is in shadow.
        let mut buried = solid_world();
        buried.snapshot_surface();
        buried.set(32, 3, AIR);
        buried.set(32, 60, AIR);

        assert!(layers(&buried, 32, 3).veils.is_empty(), "a hole near the surface is unveiled");
        let deep = layers(&buried, 32, 60);
        assert!(deep.bases.is_empty());
        let veil = deep.veils.first().expect("a deep pocket is veiled");
        assert_eq!(veil.tint, [0, 0, 0, 255], "a deep pocket shows nothing behind it");
        assert_eq!(veil.id, ID_BACKGROUND, "air stays background, so terrain against it is outlined");
    }

    #[test]
    fn every_mask_an_overlay_can_reference_exists_in_the_atlas() {
        assert!(u16::from(u8::MAX) < atlas::MASK_COUNT, "8-bit masks must be addressable");
        assert!(SOLID_MASK < atlas::MASK_COUNT);
        assert!(THIN_MASKS + u16::from(u8::MAX) < atlas::MASK_COUNT, "every thin edge must be addressable");
    }

    #[test]
    fn a_chunk_build_covers_every_tile_once() {
        let world = solid_world();
        let mut out = Layers::default();
        build_chunk(&world, 0, 0, &mut out);
        assert_eq!(out.bases.len(), crate::world::CHUNK_AREA, "every tile emits a base");
        // The chunk's outer ring borders the world edge, which reads as air.
        assert!(!out.overlays.is_empty(), "the world edge should bleed inward");
    }

    #[test]
    fn faces_to_air_carry_a_ragged_edge_on_every_layer() {
        let mut world = solid_world();
        world.snapshot_surface();
        world.set(32, 59, AIR); // north
        world.set(33, 59, AIR); // north-east only: an inner corner, not a bitten side
        let stone = layers(&world, 32, 60);
        assert_eq!(stone.bases[0].edges, N as u32 | EDGE_BLOCKY, "stone breaks blocky, on its open side only");
        assert!(stone.overlays.iter().all(|o| o.edges == stone.bases[0].edges), "rims follow the bite");
        assert_eq!(stone.veils.len(), 1, "a bitten tile deep down veils what the bite uncovers");

        world.set(32, 60, DIRT);
        assert_eq!(layers(&world, 32, 60).bases[0].edges, N as u32, "dirt crumbles");
        assert_eq!(layers(&world, 40, 40).bases[0].edges, 0, "enclosed tiles keep straight edges");
        assert!(layers(&world, 40, 40).veils.is_empty());
    }
}
