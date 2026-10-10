# Project Map

## Purpose
"abyss" — a 2D sandbox game heavily inspired by Terraria: own world setting,
crafting stations, progression, possibly distinct art style, and more dynamic,
visually intriguing combat. Built partly as a learning exercise.

## Requirements
- Good performance on Apple Silicon MacBook (modded Terraria/tModLoader performs poorly there)
- Cross-platform
- Learner-stated preferences: Rust, wgpu for rendering; ECS for entities still undecided
- Finite world size, Terraria-style (learner decision) — no infinite side-scrolling overworld
- Multiple dimensions, e.g. an overworld plus "the abyss", reachable by teleport (learner requirement)
- Tile behaviors wanted eventually: mining with per-type hardness tiers (some tiles unmineable
  until better tools), grass growth, saplings/trees, flowers, corruption-style spread,
  meteor impacts, liquids with differing viscosity
- Expandable pass-based world generation: dirt/grass/stone first; ores, structures, biomes later

## First slice (learner-defined)
Run the app -> seeded finite world generated -> tiles rendered -> player spawned and able to
traverse/collide. No menus or UI. Mobs, weapons, projectiles, particles deferred.

## Components
Implemented (Rust binary crate `abyss`, no dependencies, edition 2024):
- `src/tiles.rs` — `TileId` (u16), ids AIR/DIRT/STONE, `TILE_TYPES` (name, solid,
  atlas_mat, precedence, colour, drag) and `DECOR_TYPES` (grass), which grow on a
  tile's air-facing sides instead of being tiles themselves
- `src/world.rs` — `Chunk { ids: [TileId; 1024], decor: [u8; 1024], revision }`,
  per-column `highest_solid` (live) and `surface` (the generated terrain, frozen at
  generation), `sky_level` for light seeding and `skyness` for the backdrop gradient, `World` of 66x19 chunks
  at 2112x608 tiles; `get` (AIR out of bounds), `set` (bumps the revision of its own
  chunk and of any chunk within one tile, since borders autotile across chunks, and
  clears decoration when the tile changes), `decor`/`set_decor`
- `src/atlas.rs` — procedural atlas: 4x4-tile material blocks in a top band (sampled
  by world position so nothing repeats per tile), 257 one-tile transition masks below;
  written to and loaded from `assets/tiles.png`. Tuning knobs: `BAND`, `FALLOFF`,
  `BLOCK_TILES`, and the air-shadow colour
- `src/autotile.rs` — `Instance`, `emit_tile` (one opaque base, one overlay per
  bleeding material, one per decoration on air-facing sides, painted in precedence
  order), `build_chunk`
- `src/worldgen.rs` — `PASSES` list driving terrain then grass; seeded hash-based
  1D value noise, 4 octaves, for the surface heightmap and the dirt/stone boundary
- `src/camera.rs` — `Camera` (tile-unit centre, integer px-per-tile zoom steps),
  visible-tile rect, pixel snapping, `ViewTransform { scale, offset }`, world clamping,
  `follow` (immediate) and `follow_smoothed` (framerate-independent exponential ease
  at `FOLLOW_RATE`), `fit_view` (picks the integer px/tile step framing closest to a
  tile-count target), `view_tiles`
- `src/render.rs` — wgpu 30 setup for either a window or an offscreen capture target;
  atlas texture/sampler, alpha blending, per-chunk CPU instance caches keyed by chunk
  revision, per-frame assembly (all bases, then all overlays) into one buffer and one
  draw; `save_capture` reads the offscreen target back to a PNG
- `src/outline.wgsl` — final pass: scene colour + id textures to screen with outlines
- `src/shader.wgsl` — unit quad from `vertex_index`; samples the material cell and
  multiplies by the mask cell's alpha and the instance tint
- `src/physics.rs` — `GRAVITY`/`MAX_FALL` shared by every body, `move_box`
  (sub-stepped, x-then-y; a step is an instant lift
  gated on destination clearance alone, reported via `Contacts::stepped_up`, and
  offered via `step_available` when refused so the caller owns the policy),
  `grounded` probe, `supported`, `solid_at` (boundary is solid)
- `src/player.rs` — `Player { pos, vel, grounded, step_press, step_lag }`, `Intent`,
  gravity/walk/jump, `medium_drag` hook, `mine` with reach check, step policy
  (`STEP_DELAY` wind-up, `STEP_CATCH_UP` visual lag) and `visual_pos` used for both
  drawing and camera following
- `src/entity.rs` — `Body { pos, vel, half, grounded }` (a kinematic AABB, not a
  simulated rigid body), `Kind(u16)`, `Sprite { half, offset, tint }` (drawn extents,
  foot-aligned, deliberately larger than the hitbox), `Entities = hecs::World`
- `src/backdrop.rs` / `src/backdrop.wgsl` — parallax backdrop: `BackdropSet` (sky
  colours + 5 layers: shape, factor, baseline, height, colour, haze, drift), `FOREST`,
  silhouette strips (top, bottom per art-px column, 2048 wide, seamless), drawn by one
  full-screen pass at the start of the scene pass; layers anchored to `World::horizon()`
  (mean generated surface)
- `src/sky.rs` — `Sky { time }` on `World`, `Lighting` (toward, direct, ambient, sky
  colours, stars, sun/moon angles, phase); palette `KEYS` per time of day; moon phase
  scales moonlight
- `src/light.rs` — `LightMap`: RGB light over the view plus a margin, seeded from sky
  exposure and emitting tiles, propagated by forward/backward 8-neighbour raster
  passes (diagonals charged sqrt(2) decay, so falloff is radial not Manhattan),
  uploaded as a linearly-filtered texture
- `src/mob.rs` — `MOB_KINDS` table (stats plus a `brain` fn pointer), `BrainCtx`,
  `MobAction`, the bouncer and digger brains, the per-frame `update` system, and
  `Spawner` (cap, off-view placement on solid ground, forgetting distant mobs)
- `src/trees.rs` — `Tree` (root, width 1-2, height, full height, seed), `Forest`
  (trees, falling pieces, wind clock) on `World`; `after_cut` fells what a cut
  severs (topple about the cut at the base, straight drop above it), `update`,
  `emit` (one sheared bark quad per trunk row plus seeded branches), `anchors`
  Canopies: crown + branch-tip clumps as seeded foliage shapes; `plant`, foliage
  clearing on felling, leaf clusters drawn from the grid's foliage density
- `src/noise.rs` — `NoiseLog`, a bounded ring of recent disturbances; mining writes
  to it and diggers read from it
- `src/main.rs` — winit 0.30 `ApplicationHandler`: A/D walk, W or Space jump, left
  click mines, `-`/`=` zoom, Esc quit, camera follows the player; `--dump` ASCII mode,
  `--gen-atlas`, `--screenshot PATH` headless capture, `--frames N` smoke run, `--zoom`

Confirmed design decisions:
- Full RGB lighting from the start, smooth, no monochrome fallback for now — the
  learner's reasoning being that colourful projectiles and effects are part of the
  game's visual identity, and Rust should afford it
- The player emits no light itself; carried torches and emissive equipment will,
  once equipment exists
- The sky backdrop is a gradient with depth below the *generated* terrain, not a
  per-tile yes/no: digging never moves the horizon, and neighbouring air tiles can
  never differ by the whole range. Light seeding separately uses a neighbourhood
  test ("how much sky can I see"), so a narrow shaft is lit from its mouth and fades
- Hybrid entity handling: hecs for heterogeneous entities; particles, projectiles and
  bosses are expected to sit outside it. No scheduler — systems run in an explicit
  order so frames stay reproducible
- One cohesive `Body` component rather than separate Position/Velocity: in this game
  they always travel together, and it matches `move_box`'s signature
- No physics crate: a kinematic controller is the right model for a platformer and a
  tile grid is its own spatial index. Revisit only for genuinely physical debris,
  and then alongside rather than underneath
- Mob behaviour lives in `brain` function pointers in a static table, so adding a mob
  is one table entry and one function
- Grass is decoration on dirt, not a tile: stored as a `decor` bit per tile, seeded
  by worldgen on dirt touching air on any of its 8 neighbours (diagonals included, or
  step inner corners stay bare); freshly mined dirt stays bare until spread
- Material textures span 4x4 tiles and are sampled by world position, so no per-tile
  repeat is visible
- Rendering: single per-frame instance buffer (not cached meshes); per-chunk instance
  caching is the agreed upgrade path, same instance layout
- Zoom: integer physical pixels per tile, stepped [4, 6, 8, 10, 12, 16, 20, 24, 28, 32],
  camera snapped to whole physical pixels — chosen to eliminate edge crawl while panning
- Default framing is specified in tiles (120x70, measured from the learner's preferred
  Terraria zoom) and the px/tile step is derived per display, so framing is stable
  across the 2560x1440 monitor and the 3456x2234 Retina panel
- 1 tile = 1 world unit; player hitbox intended ~2x3 tiles
- Flat per-type colours until the texture atlas step
- Tile storage: chunked grid, struct-of-arrays inside each chunk; palette compression
  deferred as a possible later optimization
- Tile updates: a deliberate mix of strategies rather than one, chosen per behavior
- Type properties (hardness, solidity, texture, light) live in a type table indexed by
  tile id, not in per-tile storage

- Trees: trunk cells are `TRUNK` tiles (walk-through, choppable); the tree draws
  itself as one bending shape, the grid draws trunk cells as air. Small wind sway
  only, so drawing stays on the chop cells. Branches drawn only
- Felling: 1-wide trunk severs at any cut; 2-wide at any two cuts in a 2x2 window.
  Any felled part drops to the ground; 4+ rows then topples away from the chopper,
  shorter breaks on impact; breaking vanishes until items
- Foliage density is a grid layer (`Chunk::foliage`, half-tile resolution), not
  tree-owned; leaves are clusters placed from it so drawing and data agree.
  See-through tiles (glass, grates) will be a type-table property, not this layer
- Outlines: screen-space; every quad carries an object id that grows with draw order
  (background 0, terrain 1, then trees' lobes/wood, player, mobs); a pixel is
  darkened if something with a smaller id lies within ~2 art pixels. `outline.wgsl`
- Draw quads are a corner plus two edge vectors, so quads can lean and rotate

## Main Flow
startup -> worldgen passes (seeded) -> tile grid (finite, chunked, in memory)
                                            |
  held keys -> Intent -> Player::update -> physics::move_box (reads tile grid)
                                            |
                              camera.follow(player) -> visible tile rect
                                            |
                      build_instances (tiles) + player quad -> one instanced draw

Mining writes to the tile grid via `World::set`, which bumps chunk revisions; the
renderer rebuilds any visible chunk whose revision no longer matches its cache.

## Data and Trust Boundaries
Single-player assumed so far; not confirmed. Save/world storage undecided.

## Build and Deployment
- `cargo test` — 140 unit tests across world, worldgen, camera, physics, player,
  atlas, autotile and the shader guard
- `cargo run --release` — opens the window; WASD pans, `-`/`=` zooms, Esc quits
- `cargo run --release -- --dump --x 400 --w 150 --h 28` — ASCII view, no window
- `cargo run --release -- --frames 180` — renders N frames then exits (smoke check)
- `cargo run --release -- --screenshot shot.png --zoom 6` — headless PNG capture
- `cargo run --release -- --gen-atlas` — rewrite `assets/tiles.png`
- Dependencies: wgpu 30, winit 0.30, pollster 1, bytemuck 1 (derive), image 0.25
  (png only), hecs 0.11

## Measured
- Lighting at the default framing (128x72 tiles): ~750 us over a 171x115 region
- Lighting at maximum zoom-out (480x270 tiles): ~4.3 ms over a 523x313 region;
  knobs are `ROUNDS`, `MARGIN`, or computing at lower resolution when zoomed out
- World generation: ~2-4 ms for 1.28M tiles
- Frame assembly at 16 px/tile: ~16-19 us cached, 140 us when rebuilding 12 chunks
- Frame assembly at 4 px/tile, 1920x1080: ~54 us cached, 1.26 ms rebuilding 144 chunks;
  71k instances, of which 1.8k overlays

## Tuning knobs
- `world.rs`: `SKY_FADE_START` / `SKY_FADE_DEPTH` (how deep the sky backdrop survives)
- `light.rs`: `AIR_DECAY`, `SOLID_DECAY`, `ROUNDS`, `MARGIN`
- `light.rs`: `FOLIAGE_OPACITY` (how much leaves stop the sun), `BOUNCE`
- `sky.rs`: `KEYS` (the day's palette), `MOONLIGHT`, `MOON_SKY`, `ARC`, `DAY_SECONDS`
- `render.rs`: `RAY_STRENGTH` (god rays)
- `atlas.rs`: `BAND`, `FALLOFF`, `BLOCK_TILES`, air-shadow colour
- `player.rs`: `STEP_DELAY` (how eager steps are), `STEP_CATCH_UP` (how fast the
  drawn position catches up)
- `trees.rs`: `SWAY`, `TRUNK_FILL`, `TAPER`, `FLARE`, `BRANCH_THICKNESS`, `wind`;
  `worldgen.rs` tree pass: width odds, gap odds (2-3 vs 8-14)
- `camera.rs`: `FOLLOW_RATE` (camera stiffness), `DEFAULT_VIEW_TILES` (framing, 120x70)

## Unknowns
- Which update strategy backs each specific behavior (corruption, saplings, liquids, farms)
- Entity storage beyond one player: plain structs vs ECS, to be revisited when mobs arrive
- Whether transition band width and air-shadow strength need art tuning
- Mipmaps for the zoomed-out view (nearest sampling aliases at 4 px/tile)
- How offscreen/cross-dimension production is represented, and where its output accumulates
- How dimensions are represented and whether inactive ones stay resident
- ECS crate vs engine (direct wgpu + standalone ECS vs an engine bundling both) — undecided
- Save format
- Multiplayer: in scope or not

## Third-party assets (not yet used)
- `temp-assets/`: `templateforchar.png` (barebones character spritesheet template),
  `PixelFlush - Pixel Weapons Mega Pack.zip` (weapons), `items.zip` (item icons).
  Learner-supplied, for when characters, weapons and items are built. Check each
  pack's licence before shipping.
