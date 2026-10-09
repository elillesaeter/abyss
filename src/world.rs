//! The tile grid: a finite world of fixed-size chunks, struct-of-arrays inside each.
//!
//! Only tile ids are stored today. Further per-tile state (walls, liquid, flags)
//! becomes additional arrays on `Chunk` without touching existing access paths.

use crate::sky::Sky;
use crate::tiles::{AIR, TileId, is_solid};
use crate::trees::Forest;

pub const CHUNK_SIZE: i32 = 32;
pub const CHUNK_AREA: usize = (CHUNK_SIZE * CHUNK_SIZE) as usize;

/// 66 x 19 chunks. Both axes are multiples of `CHUNK_SIZE` so no chunk is partial.
/// How deep below the terrain the sky backdrop stays full, and how far it takes to
/// give way to darkness after that, in tiles.
///
/// The fade has to clear the depths a player reaches by ordinary surface digging:
/// starting it immediately turns a three-tile notch into a visibly dark patch, which
/// reads as a shadow nothing is casting.
const SKY_FADE_START: f32 = 8.0;
const SKY_FADE_DEPTH: f32 = 32.0;

/// How many columns either side must also be open for a tile to count as outdoors.
///
/// Testing only the tile's own column would call every tile of a narrow shaft open
/// to the sky, lighting a hundred-tile mineshaft like a meadow. Requiring the
/// neighbourhood to be open approximates how much sky a tile can actually see.
pub const SKY_RADIUS: i32 = 2;

pub const WORLD_WIDTH: i32 = 2112;
pub const WORLD_HEIGHT: i32 = 608;

pub struct Chunk {
    pub ids: [TileId; CHUNK_AREA],
    /// Decoration bits per tile (grass today). Separate array rather than a field
    /// on the tile: struct-of-arrays means adding it touches no existing access.
    pub decor: [u8; CHUNK_AREA],
    /// Leaf density, 0-255, at half-tile resolution: four values per tile, in
    /// tile order, each tile's four as [top-left, top-right, bottom-left,
    /// bottom-right]. Half-tile so gaps in a canopy can be narrower than a tile.
    pub foliage: [u8; CHUNK_AREA * 4],
    /// Bumped on every real tile change in this chunk or in a tile bordering it.
    /// A counter rather than a flag so consumers compare against what they last
    /// built, instead of needing a mutable world to clear it.
    pub revision: u32,
}

pub struct World {
    chunks: Vec<Chunk>,
    width_chunks: i32,
    height_chunks: i32,
    /// Row of the topmost solid tile in each column, or `height()` if the column is
    /// empty. Maintained on write so seeding daylight never has to scan upward.
    highest: Vec<i32>,
    /// The terrain's surface as generated, never updated afterwards. What shows
    /// behind a hole depends on how deep the hole is, not on whether you happen to
    /// have cleared the tiles above it — digging a shaft does not move the horizon.
    surface: Vec<i32>,
    /// Trees standing on the grid and pieces of them falling. Their trunk cells
    /// are ordinary tiles; this is what draws and fells them.
    pub forest: Forest,
    /// Time of day.
    pub sky: Sky,
}

impl World {
    pub fn new(width: i32, height: i32) -> World {
        assert!(
            width % CHUNK_SIZE == 0 && height % CHUNK_SIZE == 0,
            "world dimensions must be multiples of {CHUNK_SIZE}"
        );
        let width_chunks = width / CHUNK_SIZE;
        let height_chunks = height / CHUNK_SIZE;
        let count = (width_chunks * height_chunks) as usize;
        World {
            chunks: (0..count)
                .map(|_| Chunk {
                    ids: [AIR; CHUNK_AREA],
                    decor: [0; CHUNK_AREA],
                    foliage: [0; CHUNK_AREA * 4],
                    revision: 0,
                })
                .collect(),
            width_chunks,
            height_chunks,
            highest: vec![height; width as usize],
            surface: vec![height; width as usize],
            forest: Forest::default(),
            sky: Sky::default(),
        }
    }

    pub fn width(&self) -> i32 {
        self.width_chunks * CHUNK_SIZE
    }

    pub fn height(&self) -> i32 {
        self.height_chunks * CHUNK_SIZE
    }

    pub fn width_chunks(&self) -> i32 {
        self.width_chunks
    }

    pub fn height_chunks(&self) -> i32 {
        self.height_chunks
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width() && y < self.height()
    }

    /// Out-of-bounds reads give `AIR`, which keeps neighbour lookups at the world
    /// edge branch-free for callers (autotiling, grass checks, collision).
    pub fn get(&self, x: i32, y: i32) -> TileId {
        if !self.in_bounds(x, y) {
            return AIR;
        }
        let (chunk, local) = self.index(x, y);
        self.chunks[chunk].ids[local]
    }

    /// Writes a tile and bumps the revision of every chunk whose rendering depends
    /// on it: its own, plus any neighbouring chunk within one tile, because tiles
    /// on a chunk border autotile against neighbours across it. Unchanged writes
    /// bump nothing, so a generation pass rewriting the same value is free.
    pub fn set(&mut self, x: i32, y: i32, id: TileId) {
        debug_assert!(self.in_bounds(x, y), "set out of bounds at ({x}, {y})");
        if !self.in_bounds(x, y) {
            return;
        }
        let (chunk, local) = self.index(x, y);
        if self.chunks[chunk].ids[local] == id {
            return;
        }
        let previous = self.chunks[chunk].ids[local];
        self.chunks[chunk].ids[local] = id;
        // Decoration belongs to the tile that was there; a different tile starts bare.
        self.chunks[chunk].decor[local] = 0;
        self.update_highest(x, y, previous, id);

        let mut bumped = [usize::MAX; 4];
        let mut count = 0;
        for (nx, ny) in [(x - 1, y - 1), (x + 1, y - 1), (x - 1, y + 1), (x + 1, y + 1)] {
            let (cx, cy) = (
                nx.clamp(0, self.width() - 1) / CHUNK_SIZE,
                ny.clamp(0, self.height() - 1) / CHUNK_SIZE,
            );
            let index = (cy * self.width_chunks + cx) as usize;
            if bumped[..count].contains(&index) {
                continue;
            }
            bumped[count] = index;
            count += 1;
            self.chunks[index].revision += 1;
        }
    }

    pub fn chunk(&self, cx: i32, cy: i32) -> &Chunk {
        &self.chunks[(cy * self.width_chunks + cx) as usize]
    }

    pub fn decor(&self, x: i32, y: i32) -> u8 {
        if !self.in_bounds(x, y) {
            return 0;
        }
        let (chunk, local) = self.index(x, y);
        self.chunks[chunk].decor[local]
    }

    /// Decoration is drawn only on its own tile, so unlike `set` this bumps just
    /// the owning chunk.
    pub fn set_decor(&mut self, x: i32, y: i32, bits: u8) {
        if !self.in_bounds(x, y) {
            return;
        }
        let (chunk, local) = self.index(x, y);
        if self.chunks[chunk].decor[local] != bits {
            self.chunks[chunk].decor[local] = bits;
            self.chunks[chunk].revision += 1;
        }
    }

    /// Leaf density at half-tile cell (sx, sy), i.e. tile (sx / 2, sy / 2). Zero
    /// out of bounds.
    pub fn foliage(&self, sx: i32, sy: i32) -> u8 {
        let (x, y) = (sx.div_euclid(2), sy.div_euclid(2));
        if !self.in_bounds(x, y) {
            return 0;
        }
        let (chunk, local) = self.index(x, y);
        self.chunks[chunk].foliage[local * 4 + (sy & 1) as usize * 2 + (sx & 1) as usize]
    }

    /// Foliage is drawn by the trees, not the chunk caches, so this bumps nothing.
    pub fn set_foliage(&mut self, sx: i32, sy: i32, density: u8) {
        let (x, y) = (sx.div_euclid(2), sy.div_euclid(2));
        if !self.in_bounds(x, y) {
            return;
        }
        let (chunk, local) = self.index(x, y);
        self.chunks[chunk].foliage[local * 4 + (sy & 1) as usize * 2 + (sx & 1) as usize] = density;
    }

    /// Row of the topmost solid tile in column `x`; `height()` when the column has
    /// none. Tiles strictly above it are open to the sky.
    pub fn highest_solid(&self, x: i32) -> i32 {
        if x < 0 || x >= self.width() {
            return self.height();
        }
        self.highest[x as usize]
    }

    /// Records the generated terrain as the horizon for backdrop purposes. Called
    /// once, after generation.
    pub fn snapshot_surface(&mut self) {
        self.surface.copy_from_slice(&self.highest);
    }

    /// The row the backdrop's layers are anchored to: the generated surface's mean
    /// height, so the horizon sits where the land does on average.
    pub fn horizon(&self) -> f32 {
        self.surface.iter().map(|&y| y as f32).sum::<f32>() / self.surface.len() as f32
    }

    /// How much sky shows through air here: 1 above the generated terrain, fading
    /// to 0 over `SKY_FADE` tiles below it. A gradient rather than a switch, so two
    /// neighbouring air tiles never differ by the whole range.
    pub fn skyness(&self, x: i32, y: i32) -> f32 {
        if x < 0 || x >= self.width() {
            return 0.0;
        }
        let depth = (y - self.surface[x as usize]) as f32 - SKY_FADE_START;
        (1.0 - depth / SKY_FADE_DEPTH).clamp(0.0, 1.0)
    }

    /// Tiles strictly above this see open sky: the lowest surface among a column
    /// and its neighbours.
    pub fn sky_level(&self, x: i32) -> i32 {
        (x - SKY_RADIUS..=x + SKY_RADIUS)
            .map(|column| self.highest_solid(column))
            .min()
            .unwrap_or_else(|| self.height())
    }

    fn update_highest(&mut self, x: i32, y: i32, previous: TileId, id: TileId) {
        let column = x as usize;
        let before = self.highest[column];
        if is_solid(id) {
            if y < self.highest[column] {
                self.highest[column] = y;
            }
        } else if is_solid(previous) && y == self.highest[column] {
            // The column's surface was just removed; the next solid tile below
            // becomes the new surface. Only ever walks down from the old surface.
            let next = (y + 1..self.height()).find(|&row| {
                let (chunk, local) = self.index(x, row);
                is_solid(self.chunks[chunk].ids[local])
            });
            self.highest[column] = next.unwrap_or(self.height());
        }
        let _ = before;
    }

    /// Convenience wrappers used by capture scenes and tests.
    pub fn mine_for_capture(&mut self, x: i32, y: i32) {
        self.set(x, y, AIR);
    }

    pub fn place(&mut self, x: i32, y: i32, id: TileId) {
        self.set(x, y, id);
    }

    pub fn chunk_revision(&self, cx: i32, cy: i32) -> u32 {
        self.chunk(cx, cy).revision
    }

    /// World tile coordinate -> (chunk index, index within that chunk).
    fn index(&self, x: i32, y: i32) -> (usize, usize) {
        let (cx, cy) = (x / CHUNK_SIZE, y / CHUNK_SIZE);
        let (lx, ly) = (x % CHUNK_SIZE, y % CHUNK_SIZE);
        (
            (cy * self.width_chunks + cx) as usize,
            (ly * CHUNK_SIZE + lx) as usize,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{DECOR_GRASS, DIRT, STONE};

    #[test]
    fn coordinates_round_trip_across_chunk_borders() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        let probes = [(0, 0), (31, 31), (32, 32), (33, 31), (2111, 607), (1056, 304)];
        for (i, &(x, y)) in probes.iter().enumerate() {
            w.set(x, y, if i % 2 == 0 { DIRT } else { STONE });
        }
        for (i, &(x, y)) in probes.iter().enumerate() {
            let expected = if i % 2 == 0 { DIRT } else { STONE };
            assert_eq!(w.get(x, y), expected, "mismatch at ({x}, {y})");
        }
    }

    #[test]
    fn out_of_bounds_reads_are_air() {
        let w = World::new(64, 64);
        for (x, y) in [(-1, 0), (0, -1), (64, 0), (0, 64), (-99, -99)] {
            assert_eq!(w.get(x, y), AIR);
        }
    }

    #[test]
    fn a_write_bumps_the_owning_chunk_and_only_on_change() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        let before = (w.chunk_revision(1, 0), w.chunk_revision(0, 0));
        w.set(40, 10, DIRT);
        assert!(w.chunk_revision(1, 0) > before.0, "the owning chunk should be bumped");
        assert_eq!(w.chunk_revision(0, 0), before.1, "a distant chunk should be untouched");

        let after = w.chunk_revision(1, 0);
        w.set(40, 10, DIRT);
        assert_eq!(w.chunk_revision(1, 0), after, "rewriting the same id must bump nothing");
    }

    #[test]
    fn a_write_on_a_chunk_border_bumps_the_neighbours_that_autotile_against_it() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        let before: Vec<u32> = (0..2)
            .flat_map(|cy| (0..2).map(move |cx| (cx, cy)))
            .map(|(cx, cy)| w.chunk_revision(cx, cy))
            .collect();

        // The corner tile of chunk (1, 1): all four chunks meeting there depend on it.
        w.set(CHUNK_SIZE, CHUNK_SIZE, STONE);

        for (i, (cx, cy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
            assert!(
                w.chunk_revision(cx, cy) > before[i],
                "chunk ({cx}, {cy}) borders the changed tile and should be bumped"
            );
        }
    }

    #[test]
    fn the_column_surface_tracks_writes_in_both_directions() {
        let mut w = World::new(64, 64);
        assert_eq!(w.highest_solid(10), 64, "an empty column has no surface");

        w.set(10, 40, STONE);
        assert_eq!(w.highest_solid(10), 40);
        w.set(10, 45, STONE);
        assert_eq!(w.highest_solid(10), 40, "a deeper tile does not change the surface");
        w.set(10, 30, STONE);
        assert_eq!(w.highest_solid(10), 30, "a higher tile becomes the surface");

        w.set(10, 30, AIR);
        assert_eq!(w.highest_solid(10), 40, "mining the surface exposes the next tile down");
        w.set(10, 40, AIR);
        w.set(10, 45, AIR);
        assert_eq!(w.highest_solid(10), 64, "emptying the column leaves no surface");
    }

    #[test]
    fn the_backdrop_fades_with_depth_and_digging_does_not_move_the_horizon() {
        let mut w = World::new(64, 64);
        for y in 20..64 {
            w.set(10, y, STONE);
        }
        w.snapshot_surface();

        assert_eq!(w.skyness(10, 10), 1.0, "above the terrain is open sky");
        assert_eq!(w.skyness(10, 20), 1.0, "at the surface is still sky");
        assert_eq!(w.skyness(10, 24), 1.0, "a notch dug from the surface still shows sky");
        let middling = w.skyness(10, 40);
        assert!(middling > 0.0 && middling < 1.0, "partway down, got {middling}");
        assert_eq!(w.skyness(10, 63), 0.0, "well below the terrain shows nothing");

        // Digging a shaft must not turn its depths back into sky.
        for y in 20..60 {
            w.set(10, y, AIR);
        }
        assert_eq!(w.skyness(10, 63), 0.0, "a dug shaft does not raise the horizon");
        assert_eq!(w.skyness(10, 40), middling, "nor change the fade above it");
    }

    #[test]
    fn out_of_bounds_columns_report_no_surface() {
        let w = World::new(64, 64);
        assert_eq!(w.highest_solid(-1), 64);
        assert_eq!(w.highest_solid(64), 64);
    }

    #[test]
    fn changing_a_tile_clears_the_decoration_that_grew_on_it() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        w.set(40, 10, DIRT);
        w.set_decor(40, 10, DECOR_GRASS);
        assert_eq!(w.decor(40, 10), DECOR_GRASS);

        w.set(40, 10, AIR);
        assert_eq!(w.decor(40, 10), 0, "mined tiles must not leave their grass behind");

        w.set(40, 10, DIRT);
        assert_eq!(w.decor(40, 10), 0, "newly placed dirt starts bare");
    }

    #[test]
    fn setting_decoration_bumps_only_its_own_chunk() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        w.set(CHUNK_SIZE, CHUNK_SIZE, DIRT);
        let (own, neighbour) = (w.chunk_revision(1, 1), w.chunk_revision(0, 0));
        w.set_decor(CHUNK_SIZE, CHUNK_SIZE, DECOR_GRASS);
        assert!(w.chunk_revision(1, 1) > own, "the owning chunk must rebuild");
        assert_eq!(w.chunk_revision(0, 0), neighbour, "decoration does not cross chunk borders");
    }

    #[test]
    fn an_interior_write_does_not_bump_neighbouring_chunks() {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        let neighbour = w.chunk_revision(1, 0);
        w.set(16, 16, STONE); // middle of chunk (0, 0)
        assert_eq!(w.chunk_revision(1, 0), neighbour, "only the owning chunk depends on it");
    }
}
