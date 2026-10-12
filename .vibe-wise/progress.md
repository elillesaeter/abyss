# Learning Progress

No learning events recorded yet.

## World model and scope
Introduced (explained by Claude):
- ECS suits entity iteration; a multi-million-tile grid is normally a dense chunked array instead
- ECS vocabulary: a system is a function over component arrays; the player is an entity, not a system
- Finite whole-world generation (Terraria) vs lazy per-chunk generation (Minecraft) and their costs

Learner reasoning:
- Chose finite world, reasoning that infinite side-scrolling generation is not needed for the overworld
- Scoped first slice to seeded gen + rendered world + traversable player, deferring mobs/weapons/particles
- Proposed pass-based, extensible generation (dirt/grass/stone now; ores, structures, biomes later)
- Raised multiple dimensions as a requirement before being prompted about it
- Recognized some tiles need a hardness/tier notion for gated mining

Needs reinforcement:
- Misread an earlier point as "ECS may not fit this game"; the claim was narrower (tiles, not entities)
- Tile data content undecided; has requirements but not yet a representation

## Tile storage and update scheduling
Introduced (explained by Claude):
- Type-table properties vs per-tile state; why mining progress is transient, not stored per tile
- Per-tile byte budget multiplied by ~20M tiles, doubled by a second resident dimension
- Four storage layouts compared: flat AoS, SoA, chunked, palette-compressed chunks
- Five update strategies compared: no offscreen sim, random tick, active set/dirty queue,
  scheduled event queue, lazy catch-up by timestamp
- Why lazy catch-up works for independent per-tile processes but breaks for interacting ones
  (spread, liquids), and why timestamps belong per chunk or in a sparse side table

Learner reasoning:
- Stated lack of experience with tile-data optimization and asked for a comparison (help requested)
- Independently arrived at lazy catch-up via per-tile "last seen" timestamps, a real technique,
  before being shown it

Learner decisions:
- Chunked struct-of-arrays for tile storage, with palette compression left as a later option
- Rejected a single update strategy in favour of per-behavior choices, reasoning from the
  character of each effect: organic corruption spread, inexact sapling timing, settling liquids
- Added a requirement that offscreen (and ideally cross-dimension) production still yields results

Learner decisions (chunk size and tile fields):
- 32x32 chunks, reasoning that smaller chunks raise the odds of a uniform region and so
  compress better under a later palette scheme, while 64x64 felt too coarse
- World 2100x600, accepted snapping to 2112x608 for whole chunks
- Independently identified tile blending as central to Terraria's look, and reasoned that
  block orientation is state rather than static data and not needed yet
- Correctly reasoned that falling sand is an entity round-trip, not tile data
- Asked Claude to propose the tile field set (help requested, not withheld reasoning)

Introduced (explained by Claude):
- Autotiling via a 4-bit or 8-bit neighbour mask (47-tile blob set); Terraria caches the
  frame per tile, but cached chunk meshes make that cache unnecessary — saving 4 bytes/tile
- Texture coordinates belong in the type table; light is a separate derived buffer
- Aggregate/level-of-detail simulation for offscreen effects, resolved to tiles on visit
- Crop farms (closed-form in elapsed time) vs mob farms (no cheap closed form)
- Offscreen production needs a sink, or item entities accumulate unbounded

Implemented: world storage + seeded generation, verified by 8 passing tests.
Claude's own test bug (comparing sky-only chunk row 0 across seeds) was caught by the
test run and fixed, not the generator.

## Renderer
Introduced (explained by Claude):
- Measured magnitudes: ~8k visible tiles at 16 px/tile, ~32k zoomed out; ~390 KB/frame
  of instance data, trivial on unified memory — so throughput does not decide the choice
- Camera alignment and free zoom are free in both schemes; chunk-aligned cameras are a myth
- The real difference is cost *shape*: instancing pays a small fixed per-frame price,
  cached meshes pay spikes proportional to how much changed
- Third option named: per-chunk cached instance buffers (bounded per-frame cost, no
  rebuild unless tiles change) as the upgrade path
- Pixel-art shimmer at fractional tile sizes, and the two standard fixes
- Terraria's player hitbox is ~1.25x2.6 tiles, deliberately non-integer, because a hitbox
  exactly as wide as a corridor snags on corners

Learner reasoning:
- Analysed tile churn over the game's lifetime (low early, high with drills/rockets/
  block-eating projectiles) as the axis for choosing a draw scheme — the right axis
- Chose 1 tile = 1 unit and a ~2x3 tile player for aesthetic balance
- Wanted zoom beyond Terraria's default and insisted the camera not be chunk-constrained
- Accepted the per-frame instance buffer recommendation with flicker-free zoom required

Implemented: wgpu renderer + pannable camera. 19 tests pass; a 180-frame bounded run
drew 7322 instances at 16 px/tile and exited cleanly.

## Player physics and entity storage
Introduced (explained by Claude):
- Why simultaneous-axis resolution with smallest-penetration push-out misbehaves
  (sideways ejection while falling against a wall; snagging on floor tile seams)
- Axis-separated resolution removes the ambiguity, and step-up must live inside the
  horizontal pass, gated on grounded or a player climbs cliffs by holding a direction
- Grounded is better as a probe than a flag set during collision
- Sub-stepping below one tile is what makes the no-tunnelling assumption hold;
  Terraria caps fall speed below a tile per tick for the same reason
- ECS gives storage and composition, NOT proximity: proximity needs a spatial index,
  and in a tile game that is chunk bucketing
- Terraria uses flat 200-NPC / 1000-projectile arrays with switch dispatch; its slowdown
  is per-entity update cost and GC, not the array layout
- Chests and crafting stations are tiles with attached data (block entities), not entities
- Medium drag needs sampling, not events; enter-moments derive from frame-to-frame diffs

Learner reasoning:
- Derived x-before-y resolution independently from a concrete case (falling right onto a
  step corner should land on top, not beside), which is the correct conclusion
- Identified that tunnelling sideways depends on hitbox slack rather than collision code
- Recognised that movement needs to register non-solid overlaps (liquids, cobwebs), not
  just collisions
- Scoped ECS value correctly to high entity counts (events, modded spawn rates,
  Journey-mode sliders) rather than assuming it from the start
- Asked what replaces an ECS for proximity queries rather than assuming ECS provided it

Learner decision: plain `Player` struct for now, physics as free functions; ECS revisited
when mobs and projectiles arrive.

Implemented: physics, player, mining, camera follow. 33 tests pass; 299/300 frames
presented in a bounded run with the player resting exactly on the generated surface.
Two Claude test bugs caught by the run and fixed: a stale shader attribute location, and
an airborne step-up test whose geometry never overlapped the step (now asserts both
sides of the gate).

## Texture atlas and autotiling
Introduced (explained by Claude):
- Three autotiling schemes and their art cost: blob bitmask (47 images), dual grid
  (16), base + overlay (1 per material plus per-pair edges)
- Only base + overlay produces material-to-material bleeding; the other two answer
  "same or different" and leave hard boundaries underground
- Per-frame instancing moved the mask cost from build time to frame time, which is
  what made the per-chunk cache worth doing now
- "Per-chunk cached instances" forks into per-chunk GPU buffers (many draws, buffer
  lifecycle) vs per-chunk CPU lists copied into one buffer (one draw, one memcpy)

Learner reasoning:
- Chose base + overlay by asking which scheme minimises artwork while contributing
  most to the look, and justified it with moss on stone: overlays make art cost
  additive (materials + decorations) rather than multiplicative
- Asked for corners to be complete from the start rather than left rough
- Proposed generating the atlas procedurally *and* committing it as a project file,
  so it is editable later without blocking on art now
- Named an art direction reference (Starlight River / late Calamity: higher fidelity,
  less vanilla) while acknowledging the visual identity is not yet decided

Learner decisions: per-chunk cached instances, base + overlay with full corners,
procedural atlas committed to `assets/tiles.png`, `image` dependency accepted.

Implemented: atlas, autotiling, per-chunk caching, offscreen capture. 47 tests pass.
Claude deviated from the proposed atlas layout (13 pre-rotated cells per material,
up to 8 overlay quads per tile) in favour of one shared mask set addressed by the
8-neighbour bitmask and one overlay quad per bleeding material, because additive
per-edge quads double-blend where they overlap. Reported at the time.

## Tiling artefacts and grass as decoration
Learner review of the first atlas result found two faults Claude had missed:
- Tiles visibly did not blend: each cell repeated identically and carried a per-cell
  vertical gradient, drawing a grid line at every tile boundary
- Grass should be dirt with grass growing on it, not solid grass blocks

Introduced (explained by Claude):
- Why the grid appeared: anything keyed to a tile's own pixel coordinates repeats
  with period 1 tile; the fix is a material texture spanning several tiles, sampled
  by world position
- Derive-vs-store for grass, and the visible consequence: derived grass makes freshly
  mined dirt instantly green

Learner reasoning:
- Specified the Terraria model precisely: grass only on surface dirt initially, spread
  during gameplay by planting, and dirt exposed in air never inherently grassy — which
  is the stored model, chosen for gameplay reasons rather than cost

Implemented: 4x4-tile material blocks, grass as a `decor` bit, mask plateau tuning.
53 tests pass, including guards that material textures neither repeat per tile nor
band vertically, that buried marked dirt shows no grass, and that mining uncovers
bare dirt.

## Feel fixes before mobs
Learner review found three faults:
- Grass broke at the inner corners of steps. Cause was the worldgen rule, not the
  renderer: marking only dirt with air *directly above* leaves the tile under a step's
  lip bare, since it touches air only diagonally. Fixed to 8-neighbour adjacency.
- Step-up read as a teleport and fired too fast to stand beside a step and dig.
  Replaced the instant one-tile lift with a rate-limited climb (`STEP_SPEED`).
  Gating it on `grounded` then broke it — a climb leaves the ground, so the body
  oscillated at the foot of the step; a `supported` probe with step-height tolerance
  is the correct gate.
- Camera was welded to the player. Added framerate-independent exponential easing.

Learner decision: move to mobs and combat next.

Learner also measured their preferred Terraria framing (about 120x70 tiles, 60-70 each
side and 30-35 above/below the player) and gave it as a requirement. Claude explained
that a fixed px/tile ladder cannot honour a tile-count spec across displays, and made
zoom derive its integer step from a tile target instead. Measured: 2560x1440 -> 20
px/tile framing 128x72; 3456x2234 -> 28 px/tile framing 123x80.

## Step-up rework
Learner found that a step that should be climbable was refused inside a mined pocket.
Claude's first two hypotheses (direction asymmetry, low ceiling) were both disproved by
tests that passed. The actual cause: the rate-limited climb re-tested a full step's
clearance from each newly-raised position, so completing a one-tile climb demanded
close to two tiles of headroom and stalled partway in any tunnel just tall enough to
walk in. Reworked so physics does an instant lift gated on destination clearance only,
and the player owns the feel: a wind-up before the step fires, and a drawn-position lag
that catches up afterwards. Camera follows the drawn position.

## Entity storage and mob AI
Introduced (explained by Claude):
- Honest for/against on ECS: composition and queries vs Rust cross-kind borrow
  friction, scattered behaviour, archetype churn, and bosses being state machines
  that ECS does not help with
- Typed struct-of-arrays pools as the alternative, with identical cache behaviour
- Why a physics crate is the wrong tool here: platformer movement is deliberately
  unphysical (hence every engine shipping a separate kinematic controller), and a
  tile grid would need per-chunk collider generation kept in sync with every edit
- What bevy_ecs adds over hecs (scheduler, resources, events, change detection) and
  why the main attraction is the one feature being declined
- Combat: the look the learner wants is mostly light, trails, hit-stop and bloom —
  renderer work, not combat code; telegraphs are what make dense patterns fair;
  ordinary mobs get interesting through terrain interaction and reacting to the
  player rather than individual depth

Learner reasoning:
- Chose the hybrid explicitly, agreeing on which entities should sit outside the ECS
- Challenged `Velocity` as an unnatural component, preferring a cohesive body, and
  asked why no physics crate. Claude agreed on the first and changed the proposal to
  a single `Body` component
- Clarified that "dynamic combat" meant projectile and effect *visuals* plus boss
  pacing, telegraphed mechanics and world manipulation, and raised the balance
  question of the world being destroyed beyond playability

Implemented: hecs store, `Body`/`Kind`/`Tint`, `MOB_KINDS` with brain fn pointers,
bouncer and digger brains, noise log fed by mining, spawner with cap and forgetting.
76 tests pass. Claude hit hecs 0.11's query shape (items come back without an entity
id unless `Entity` is a query term) and corrected it against the vendored source.

## Proportions and lighting
Learner observed that everything looked tiny next to Terraria, and separately that
the whole world being lit made it look wrong. Claude's analysis: we were drawing
collision boxes rather than sprites (Terraria's player sprite is 40x56 px against a
20x42 hitbox), and the frame is empty of the trees, walls and tile detail that give
scale. Learner asked whether sprite data belonged in `Body` or beside it; Claude
argued beside, because drawing data will grow while `Body` will not, and a sprite
needs a foot anchor rather than sharing the body's centre.

Learner decisions: full RGB lighting immediately rather than monochrome first,
smooth rather than blocky, player not self-illuminating but equipment later.

Implemented: `Sprite` component, RGB light map with directional sweeps, torch tile
and placement, air drawn as sky or void by exposure. 89 tests pass.
Three faults found during the work: a committed atlas from an older layout silently
corrupted every UV (now checked on load), a uniform struct missing the padding WGSL
inserts before a vec2, and a backdrop using the neighbourhood sky test, which painted
the air above every step of a slope black.

Learner then spotted a hard vertical seam inside a dug shaft and asked whether light
should bleed around corners. Claude tested that first: light does turn corners (the
sweeps handle it), so the fault was elsewhere. The real cause was the backdrop being
a binary sky/void choice keyed on live column state, which made a dug shaft show sky
beside untouched rock showing none. Replaced with a gradient keyed on the terrain as
generated, which also removed the void material and a chunk-invalidation rule.

Learner then reported that lighting looked wrong after mining. Claude tested the
stated claim first and it held — recomputation works, with measured values — so the
fault was elsewhere. Measuring the falloff *shape* found the real defect: carrying
light only along the axes made it Manhattan, so a torch lit a diamond and the lit
band along a slope swung with the terrain angle. Replaced with forward/backward
8-neighbour raster passes charging sqrt(2) for diagonals. Round count was then
settled by measurement rather than assumption: a tunnel climbing up-and-right leaves
its top at 0.00008 with one round and 0.0175 with two, with three identical to two.

Learner then asked whether darkness behind freshly mined surface blocks was correct
or a fault, noting it was counter-intuitive. Measurement showed two causes
multiplying: the backdrop fading 10% per tile below the generated surface, and
daylight reaching a notch only by propagation because the neighbourhood sky test
counts a narrow hollow as indoors. A three-tile notch ended at 53% of open sky.
Delaying the backdrop fade (full for the first 8 tiles, then over 32) and softening
air decay brought it to 73-89%. The learner's intuition was right in both reports.

Learner confirmed the lighting now looks right (screenshot review).

## Trees and sunlight
Introduced (explained by Claude):
- Propagation light has no direction, so it cannot form shafts; screen-space rays
  (visual only) vs a directional sun pass (real light) compared
- A canopy drawn as a sprite still needs a transmission shape in the light grid, and
  sub-tile gaps need a finer sun-pass resolution than one value per tile
- Sky light splits into direct sun (directional) and ambient skylight (propagated)

Learner reasoning:
- Specified Terraria-style trees: a sprite-looking tree with choppable trunk tiles,
  cut at any height with everything above falling, canopy not choppable
- Chose a directional sun pass for real light at the surface, and a day-night cycle
  with a moving sun

Learner reasoning (round 2):
- Proposed a canopy-climbing accessory (from a Minecraft mod) and canopies thinned by
  trampling/climbing, tying into environmental destruction; worried this forces
  blocky tile canopies, which they reject
- "Dynamic" = sway including the trunk (not a detached swinging canopy), plus strong
  per-tree variation; no seasons
- Chose build order: trunks, then canopies, then sun pass and day-night, so each stage
  has what the next needs

Introduced (explained by Claude):
- Simulation shape vs drawn shape: a hidden per-cell foliage density could drive
  collision, light and trampling while the canopy is drawn freely; they must agree
- Trunk sway means drawing a trunk as one deformed piece over tile collision data;
  small tip sway keeps drawing and collision aligned

Learner decisions (trunk slice):
- No default collision with trees; the player walks in front. Climbing later via accessories
- Small wind sway only, keeping drawing aligned with the hidden collision/chop tiles
- Mixed 1-2 tile wide trunks with varied branch configurations
- Felling: cut at the base topples; cut partway, the top drops straight down
- Distributed spacing: some trees 2-3 tiles apart, most ~10, a little denser than Terraria

Fact raised by Claude: mining refuses non-solid tiles, so "breakable" must become a
type property separate from "solid".

Learner decisions (round 3):
- Two-wide trunks fell when two cells within one 2x2 window are cut, including a
  vertically stacked pair (chosen over the pure connectivity rule)
- Branches are drawn only, not choppable; may become climbable
- A landed tree vanishes until items exist (later: wood pieces, as in Terraria)
- Trunk height 7-20 tiles, slightly taller than Terraria

Implemented (trunk slice): TRUNK tile (non-solid, breakable; grid draws it as air),
`breakable` type property, rotatable quads (pos + two axis vectors; atlas block cell
packed into the material field so rotated bark doesn't flicker), bark material,
`trees.rs` (Tree, Forest, felling, topple as a rod about its cut, straight drop,
wind sway), worldgen tree pass. 108 tests pass; 180-frame windowed run clean;
headless capture shows trees rooted on grass along slopes.
Claude-added, beyond agreed scope: the ground tile under a standing trunk cannot be
mined (by player or digger mobs), as in Terraria, so trees never float. Reported for
learner to keep or change.

Learner review: trees look good (art to improve later); a base-cut topple hinged on
the air left by the cut. Claude offered settle-then-topple, hinge at ground, or hinge
on the stump edge; learner chose settle-then-topple. Implemented: the felled part
falls straight onto whatever is under the cut, then tips about its foot. 109 tests pass.

Learner then generalised the rule: wherever a tree is cut, the felled part drops
straight to the ground; if it is 4+ rows long it then topples, otherwise it breaks
on impact. Implemented by replacing the topple/drop split with one falling model.
110 tests pass. Flagged: a mid-trunk cut falls through its own (non-solid) stump to
reach the ground, so the piece overlaps the stump until it tips.

Learner confirmed the falling rule looks right, and proposed (for later) that falling
trees damage mobs and possibly the player, fitting the actions<->environment theme;
asked how falling trees are implemented and whether they'd be projectiles.

Introduced (explained by Claude):
- Falling trees are a plain animation struct in `world.forest.falling`, terrain-only
  collision, invisible to everything else
- A projectile is a damage contract (hitbox, damage, friendly/hostile, immunity
  timers), not a motion type; rotated shapes need line/segment hit tests, not AABBs
- No health system exists yet; tree damage should plug into the shared combat
  damage path rather than get its own

Learner decisions (falling-tree damage, for the combat design):
- Trees hurt everyone, including the player who felled them; fairness comes from
  trees always toppling away from the feller, so being hit is avoidable
- Damage scales with the speed of the part that hits
- A tree toppled over a ledge keeps falling until it hits the ground, gaining speed;
  a long drop can deal very heavy damage, even one-shot the player
- Players and mobs hit by falling trees (and later other debris) get a very short
  stun: punishing but not unfair or stun-locking
- Environment-affecting mobs (e.g. diggers) could fell trees onto the player

Introduced (explained by Claude):
- Current topple ends at 90 degrees even mid-air over a ledge; the ledge case needs
  a free-falling, spinning body once the trunk leaves its hinge
- Stun-lock prevention via a post-stun immunity window
- Diggers currently cannot fell trees: trunks are non-solid (never dig targets) and
  the ground under trunks is anchored

Learner decisions: build ledge falling now, then canopies; mobs felling trees and
stun rules deferred until combat/buff systems exist.

Implemented: falling pieces have three phases — Drop (straight onto ground), Hinged
(tips about its foot), Free (released at horizontal with the centre's hinge velocity
and spin, falls under gravity until any part past its first tile touches terrain).
`speed()` reports the fastest point's speed for future damage scaling. 111 tests pass,
incl. a 20-tile cliff test (falls free, builds >25 tiles/s). Known ceiling: the hinge
stays at the foot, so a trunk whose middle catches a ledge lip breaks there instead
of tipping over it.

Learner reasoning (canopies):
- Foliage at branch tips plus a crown
- Initially wanted foliage data off the tile grid, then noticed glass/grates also
  transmit light and blocks may degrade, and was unsure

Introduced (explained by Claude):
- Type property (glass transmits light: type table, zero per-tile cost) vs per-cell
  state (foliage density, trampling, block damage: per-cell storage)
- Grid layer vs per-tree storage for per-cell state, and that per-tree storage gets
  rebuilt into a grid by anything reading it per cell
- Data resolution bounds the smallest light shaft; finer needs a finer light grid (~4x
  sun-pass cost); no save format yet, so resolution is cheap to change later

Learner decisions:
- Foliage density as a grid layer (chunk array), reasoning underground tree types may
  use it, so it is not all wasted
- Leaves drawn as clusters placed from the foliage data (Claude's recommendation,
  on request)

Learner decision: 2x2 foliage per tile from the start, for finer light shafts; a
per-tile mode may become a performance setting later.

Implemented (canopies): per-chunk `foliage` layer (u8 density, 4 per tile), crown +
branch-tip clumps as noise-broken ellipses from the seed, `plant` writes them (max
where crowns overlap), felling lowers affected cells to what remaining trees provide
(never raises), leaves drawn as rotated, tinted ~1-tile clusters per foliage cell with
a new leaf material and leaf mask, falling parts carry their leaves. 116 tests pass.
Measured: frame assembly ~0.3 ms at default zoom, ~0.9 ms at max zoom-out (was
~0.05 ms) — foliage shapes recomputed per frame; caching them is the upgrade.
Observed: crowns are dense; interior gaps exist in the data only at edges and are
mostly hidden by overlapping leaf clusters — to tune with the sun pass.

Learner observed the world still looks flat and proposed Terraria-style outlines,
worried leaf clumps would blacken whole canopies.

Introduced (explained by Claude):
- Terraria's outlines are baked into tile frames and sprites, not a shader
- Art-baked vs screen-space id-buffer outlines; "outline belongs to the object in
  front" via ids that grow with draw order
- Coloured (dark-hue) outlines as a common way to make the style its own
- Parallax backgrounds as the other half of the "flat" look

Learner decisions:
- Mobs keep their own outline in front of trees
- Canopies as 2-3 outlined lobes, partly in front of the trunk, for depth
- No outline where blocks blend (all terrain blends for now; no clay-style exceptions)
- Terraria as the outline reference, while aiming for a distinct, shippable look

Implemented: per-quad object id (terrain 1, background 0, objects from 2 in draw
order), scene pass to colour + R32Uint id targets (bases/objects write ids with
alpha<0.5 discarded; soft overlays masked off), full-screen outline pass darkening
pixels with a smaller id within ~2 art pixels (disc), near-black unlit. Crowns split
into back lobe(s) / front lobe; branch clumps are front lobes. Leaf clusters enlarged
to close pinholes that outlined as speckle. 118 tests pass; captures checked at two
zooms.

Learner compared an in-game screenshot with Terraria and judged ours to look much
worse even with outlines. Claude's analysis, ranked: (1) art — Terraria is hand-placed
pixel art on a fixed 16px-per-tile grid; ours is procedural noise, and rotated leaf
quads break the pixel grid; (2) no parallax background/sky detail; (3) terrain is a
smooth 1D heightmap staircase with no caves/overhangs; (4) no surface detail (grass
blades, flowers, vines, background walls); (5) zoomed-out framing over black.
Framed as: we have built engine systems, not look; look comes from art, backgrounds,
worldgen.

Learner decisions: no hand-made art (no experience); go procedural for now. Parallax
background first, to make the world feel complete.

Introduced (explained by Claude):
- Parallax factors and atmospheric perspective (far layers lighter/bluer/lower contrast)
- Procedural art reads well when it keeps a fixed pixel grid, a limited palette per
  material, and deliberate top-light; current tiles/leaves break all three, so the
  background is the template for redoing them

Learner decided (parallax): layers back to front = sky gradient, clouds, far mountains,
more clouds (for depth), mid forest, near hills. Backgrounds differ per biome and per
underground depth layer (layer names to be invented later), like Terraria, to match
each biome's vibe.
Learner decided: vertical behaviour A — layers anchored to a horizon line, each moving
vertically at its own parallax factor, fading out underground with the existing sky fade.
Claude suggested backgrounds as data sets selected by (biome, depth zone) and crossfaded;
only the forest-surface set is built now since biomes/caves don't exist.
Implemented (approved): src/backdrop.rs (BackdropSet data, FOREST set, Rust-generated
silhouette strips at 16 art px/tile, atmospheric ramp) + src/backdrop.wgsl (full-screen
pass, banded sky, layers composited back to front, horizon-anchored parallax, clouds drift
on the wind clock). Air tiles are now a black veil (1 - skyness) over the backdrop.
Learner asked: starry night sky -> answered: a backdrop layer (factor ~0, behind far
clouds) faded by time of day; added with the day/night cycle.
Known: dug holes near the surface show the backdrop's hills (no background walls yet);
crossfading between sets waits for a second set; backdrop unlit by the light map.

## Day/night cycle with lighting
Learner approved the parallax as is. Next: day/night cycle with lighting.
Learner decided: full directional sun lighting now. Terraria cycle length (24 min: 15 day,
9 night), tune later. Orange/pink sunsets; sun shining through canopies at sunrise/sunset;
god rays; stars; moon size affects night darkness. Volumetric clouds and fog: later.
Introduced: direct (sun, sharp, directional, blockable) vs ambient (sky, soft) light.
Learner decided: moon "size" = phase. Night should be dark: new moon ~barely visible
without a light, full moon fairly visible with clear moon shadows (a real-life winter
full-moon memory). A second, irregular factor for night light (e.g. cloud cover) may come
later.
Implemented: src/sky.rs (24-min cycle, sun/moon arcs, palette keyframes, 8 moon phases),
Bresenham sun sweep in light.rs (half-tile, foliage-attenuated, faces of solids lit,
not spread; ambient + bounce spread), direct light added in shader from its own texture,
god rays in the outline pass (scaled by brightness cubed so moonlight barely shows them),
backdrop sky/sun/moon/stars/tinted layers, --time/--moon flags, hold T = 60x time.
Lesson: blending two upstream cells per step blurred shadow edges over ~10 tiles
(compounding interpolation); stepping on a shared Bresenham pattern keeps them sharp.
Known: light cost 0.53 -> 0.79 ms; god rays are uniform haze over all lit air (beams
read only beside shadows); no clouds shading the ground; no volumetric clouds/fog.

## Fanned god rays
Learner noticed: beams at the screen edges don't point back to the visible sun; asked
whether to fan them across the screen.
Introduced: crepuscular rays look fanned from the sun because of perspective, even
though sunlight is parallel; a side view has no depth, so the fan must be faked.
Learner decided: A, fan only the visible beams; ground light/shadows stay parallel.
Implemented: outline pass marches 40 samples (30 tiles) toward the sun's screen position
(backdrop::body_centre, shared with the backdrop), counting drawn objects (ids) as
blockers, open share raised to the 6th power. Lessons: averaging the line blurred gaps
away (fixed by the power); using the parallel shadow texture as the mask smeared its
straight columns across the fan (fixed by using only ids).
Known: noon beams are subtle; RAY_STRENGTH / RAY_SHARPNESS / RAY_REACH are the knobs.
Learner decided: keep subtle midday beams, dramatic mornings/evenings (accepted as is).

## Procedural art pass
First commit made (c074241) at the learner's request; temp-assets/ and .DS_Store left
untracked (third-party packs, unknown licences).
Learner chose the art pass next.
Learner decided: terrain first; muted and earthy, slightly warm but dangerous; snap
leaves to the grid; pebbly and chunky first (pivot later if it looks weird).
Introduced: clusters not specks, hue-shifted shade ramps, every shape lit from above,
hard jagged edges instead of soft blends; identity comes from palette and shape language.
Implemented: 5-shade ramps (dirt/stone/grass), pebble generator (jittered grid, lumpy
stones, lit tops, shaded undersides, dark gaps/cracks, wraps), grass blade texture, 8x8
material blocks, hard ragged transition masks (BAND 7, FRINGE 2), thin rim (top, warm)
and lip (sides/bottom, dark) masks for faces to air, 4 leaf clump variants on the pixel
grid (no rotation, snapped positions and sway).
Tuning along the way: grass and terrain too dark under the lighting (ramps raised a
step); small leaf clumps left pinholes that outlined as speckle (clumps kept full).
Tried and reverted: letting direct light soak into ground (no visible gain).
Known: bark and branches unchanged (tree pass); grass is a band on the tile, no blades
overhanging above it; leaf clumps read as solid blobs (no inner shading).

## Ragged silhouettes
Learner approved the pebbly/chunky look. Raised: tile edges against air are straight
and sterile; Terraria's are slightly jagged. Asked whether to make them jagged.
Introduced: silhouette vs interior (outline traces the silhouette, so straight tile
edges get drawn as ruled lines); repetition risk of per-bitmask masks; option of a
world-keyed edge in the shader.
Decided (learner): B (world-keyed in shader), subtle 1-2px bites, nicked outer
corners, stone blockier than dirt.
Introduced: bitten pixels underground would expose the bright backdrop, so edge
tiles need their own veil drawn under them (veils move before bases).
Implemented (learner said "go ahead"): Instance.edges (N/E/S/W air bits +
EDGE_BLOCKY for stone) on base and all overlays; shader bites 0-2 px keyed by world
art column/row (dirt per pixel, stone in 3-px runs), nicks outer corners (dirt
diagonal 1-2 px, stone 2x2 notch), shifts mask lookup so rim/lip/grass follow.
Exposed solid tiles emit their own veil; veils now drawn before bases.
Bug hit: new field placed before `id` broke attribute order (ids read edges ->
terrain counted as background, god rays over ground). Learning point: vertex
attribute order must match struct layout.
Verified: 139 tests, noon close-up, carved underground pocket (bites veiled).
Stage: done, committed 196650f

### Ragged silhouettes, round 2
Learner review: stone spot on; dirt/grass too busy; trees hover over bites.
Decided: dirt in 1-2 px lumps (pair split/merge by hash, ~62% flat); trunks sink
ROOT_SINK 2/16 tile; outline skips terrain directly below an object (roots and
feet blend into ground), learner chose this over a trees-only rule.
Introduced: the outline as a depth rule ("edge where something meets what is
behind it") and when that rule gives the wrong visual answer.
Verified: 140 tests; close-ups of trunk base and player feet show no sole.
Stage: done, committed 196650f

## Tree art pass + toppling feel
Learner asked for the tree art pass, and raised two toppling issues: a snap of a
few degrees before the fall starts, and the fall being too fast.
Diagnosed: Falling::new starts at angle 0.05 rad (~3 deg) instantly (snap; set so
the pendulum doesn't stall at 0); hinge uses world GRAVITY 45 tiles/s^2, tuned for
jumping, about 2-3x "real" for a tree's size.
Introduced: game gravity vs per-object time scale; a chop "nudge" (initial spin)
instead of an initial angle.
Learner decided: fall ~2 s with a brief creak; bark vertical with a few knots;
lit slightly from the sun side; branches and roots fork; no varieties yet; all
trees taller (mighty forest). Canopy style unanswered -> default: clumps lit on
top, shaded underneath.
Implemented: Creak phase (0.35 s dying wobble <=1 deg) then NUDGE 0.7 rad/s from
upright; TOPPLE_GRAVITY 25 (python sim showed old 14-tall fall was already ~1.9 s;
the felt "too fast" was the end whip, old end spin 3.05 rad/s vs now 2.4).
Heights 10-28. Bark: wandering 1-px grooves + knots, periodic over the block
(atlas cached in assets/tiles.png; regenerate with --gen-atlas). LIT_SIDE edge
bit: direct light x(1 + 0.3 * facing) across the wood; sun x passed in direct.w.
Branches = stem + 2 twigs each with a clump. Roots: 2-3 a side, two tapering
pieces, some forked, drawn with ID_TERRAIN (first try was outlined dark bars
inside the dirt; as "ground" they blend into dirt and outline against sky).
Limb quads put the across axis first so grain runs along them.
Stage: implemented, 144 tests pass; awaiting learner review (not committed)
Review round 1 (learner): roots "look shit" — wants them like a reference
illustration (thick flare from trunk, smooth tapering roots forking 2-3 levels
into fine curling tips), just shorter. Creak wobble looks janky (it's ~4 Hz shake,
~8 px at the crown of a tall tree).
Learner chose A (outlined roots) + creep; said implement.
Round 2 done: Creep phase eases lean 0->0.09 rad over 0.4 s (t^2), hands hinge
the matching spin; TOPPLE_GRAVITY 40. Roots: root_starts (2 lateral, 1-2 down)
+ recursive grow(): chains of 0.15-tile pieces tapering, curling 0.06 rad/piece,
forking twice. Outline pass couldn't outline them evenly (it skips edges with
ground below, so roots were outlined on top only) -> roots keep ID_TERRAIN and
draw their own 1-px dark rim (all rims first, then bark, so forks merge).
Concept taught: shapes from chains of small pieces; self-drawn rims vs outline pass.
Round 3 (learner: bark too flat vs pebbly terrain): bark now a 5-shade
BARK_RAMP like dirt/stone; bark_shades() builds long narrow plates between
wandering grooves, cracks across, bevel (lit top+left, shaded bottom, right left
to the groove), 7% pits, knots with lit upper-left rim. First try (5-14 px plates,
all four sides bevelled) read as brickwork.
Stage: done, committed. Learner happy ("for procedurally generated textures that looks a lot better").

## Atmosphere + backdrop refresh
Pushed 93c179e first. Learner: atmosphere next (deferred: volumetric clouds, fog,
cloud shadows on ground) + backdrop too basic/pixelated: sun, moon, sky gradient
has too few steps.
Diagnosed: backdrop.wgsl SKY_BANDS 8 hard bands over SKY_SPAN 48 tiles (deliberate
"painted pixel art"); sun/moon drawn on the 16-px art grid, hard disc + two hard
halo rings (1.6r, 2.6r); moon flat single colour; layers 3 flat shades each.
Learner decided: 1 C (in between: smooth light, pixel shapes); 2 sun glow tinting
sky (warmer/bigger low) + moon with maria/craters and soft terminator; 3 haze;
4 clouds (lit tops, shaded undersides) + haze + morning mist; no proper fog, no
cloud shadows.
Implemented: smooth sky (pow 0.8) + horizon haze band + sRGB-space dither; sun
at screen res with exp glow (wider when low) and warm sun-side sky at dawn/dusk;
moon at screen res with soft terminator, value-noise maria, 5 fixed craters,
halo by fullness, dark side = starless sky; foot haze per layer (mix to the sky
behind, thickest at the foot); morning mist (Lighting.mist: gathers last 6% of
night, gone by ~8:00) as drifting noise bands around treeline/hills bases, pale;
clouds shaded cool underside -> lit top band following the silhouette, sun-facing
edge brighter. First mist try filled everything below the treeline (no lower
bound) and was too pink.
Not unit-testable (shader): moon/sky banding checks done by screenshots only.
Stage: implemented, 147 tests pass; awaiting learner review (not committed)
Review: learner "stunning". Night fix approved & done: haze was baked on CPU
toward the fixed daytime horizon then scaled by moonlight -> pale glowing
mountains at night. Now shader: mix(shade*light, current sky_horizon, haze).
BackdropSet.sky_horizon removed. Clouds still too bright at night (rework next).
New request: cloud variation (size + shape), less opaque ("white sprites, not
volumetric vapour").
Learner: clustered like tree canopies, variable distance/shape/size; far = soft
distant haze; cloud count to be driven by a weather system later (not yet).
Implemented: Shape::Clouds{wisps, soft}; CPU cloud_plan (clusters, heaps/banks/
wisps, sizes skewed small) + cloud_density (soft-union blobs, noise erosion,
wraps) -> R8 texture 2048x(2x192), binding 2. Shader: manual bilinear vapour(),
5 samples toward the light for self-shadow (exp(-0.35*sum)), alpha
smoothstep(0,.9,d)*0.85, far layer sampled smoothly, near on art-pixel centres,
dimmed by stars at night, haze toward current horizon. First pass: 95% coverage
(gaps widened), cores too opaque, heap had a waist (rebuilt as dome).
Stage: done, committed.

## Character sprite + animation
Learner supplied temp-assets/templateforchar.png (their template; licence to
confirm before shipping). 288x256 = 9x8 frames of 32x32; figure ~16x22 px.
Rows 0-1 face right, 2-3 front, 4-5 left, 6-7 back (top-down RPG template);
side view uses rows 0-1 (+4-5 or mirroring). Row 0: idle, 4 walk, turn, arm-out
(use) frames; row 1: crouch/jump-ish, lying, flying/swimming poses.
Player today: flat-colour box 2x3 tiles (32x48 art px), hitbox 1.5x2.6.
Key tension: 1:1 pixels -> figure ~1x1.4 tiles (half today's size, hitbox
must shrink); 2x -> matches size but pixels twice the world's.
Learner decided: 1:1 pixels (small hitbox), anims idle/walk/jump/use in that
order, mirror for left, turn toward cursor on actions; sheet is a free itch.io
placeholder, licence unchecked, never commercial.
Implemented: sheet read at runtime from temp-assets (never baked into the
committed assets/tiles.png), rows 0-1 copied into a 32-px sprite strip at atlas
y 656; SPRITE edge bit makes the shader take colour+alpha from the frame (mat =
frame index); mirroring = negative axis_x. HALF 0.375x0.65 (figure 12x21 px,
feet 5 px above frame bottom). Player: facing, walked (distance, grounded only),
using timer; frame() picks USE > RISE/FALL > WALK(by distance, STRIDE 0.35) >
IDLE. Clicks call act_toward(cursor x). Box fallback if sheet missing.
Learner review: figure far too small next to the world. Itch sheet dropped.

## Generated paper-doll character
Learner decided: option 1 (pixel frames painted by code at startup), ~3 tiles
tall like Terraria, pixel art + smooth lighting, customisable skin/hair/eyes/
clothing (character creation later), armour layers later.
Proposed: 40x56 frames, 10 grayscale layers (back hand, back sleeve, body
skin, pants, shoes, shirt, eyes, hair, front hand, front sleeve) tinted per
instance by an Appearance; one Pose per frame drives every layer painter;
15 frames (idle, 8 walk, rise, fall, 4 use); hitbox 1.25x2.75; itch sheet code
removed.
Implemented: src/character.rs (Part, PARTS, Appearance, Pose -> Skeleton, Canvas bar/block painters, paint() at startup into band at y 656, 10 rows x 40x56 frames, mat = row*32+frame); autotile::character returns 10 tinted instances sharing one id; HALF 0.625x1.375; STRIDE 0.3; itch sheet loader removed. Idle has no breathing bob (single frame).
Stage: implemented, 160 tests pass; awaiting learner review in-game (not committed)
Learner review (iteration 1): feet cycle too fast; arms straight forward when
falling look odd; looks nothing like Terraria, far too skinny everywhere; looks
small in the world. Wants fullscreen + closer zoom by default (always zooms in
once and fullscreens). Cause of the framing: view is fitted to the 1280x720
window at startup, fullscreen then keeps px/tile so ~25% more tiles show.
Learner decided: 1:1 pixels with chunky Terraria proportions, Terraria eyes
(white + iris), fullscreen + ~100 tile framing.
Implemented iteration 2: head 16x15, torso 15 wide, legs r3.2 (6+6 long),
sleeves r2.8, hands r2.4, fuller hair, EyeWhites part (always white, 11 parts),
STRIDE 0.5, fall arms up/out. Camera remembers a fitted framing and refits on
resize until zoomed by hand; DEFAULT_VIEW_TILES 100x58; borderless fullscreen
at startup.
Learner approved (good for the world scale). Stage: done, committed
Learner review (iteration 2): much better; legs still too skinny; character
feels short for the world - wants it a bit taller even if it needs a 4-tall
gap. Idea for later: Terraria-style smart cursor for digging shafts.
Learner chose 3.25 tiles. Implemented iteration 3: frame 40x64, HALF
0.625x1.625 (needs a 4-tall gap), head unchanged, torso +3 (29..42), hip->ankle
17 (thigh/shin 8.5), legs r4.0 thigh / r3.6 shin, STRIDE 0.65. No standing gap
between legs (two 8 px legs don't fit apart under a 15 px torso).
Learner approved (good for the world scale). Stage: done, committed

## Combat + gel cubes
Learner decided: sword swings in a Terraria-style arc (blade hits what it
sweeps); hotbar + inventory, with pickaxe/axe in a separate tool belt used by
a smart cursor (not taking hotbar slots); slimes hurt the player (contact
damage, knockback, i-frames, hearts, respawn); hit feel = knockback + damage
numbers (goo particles later, no particle system yet); slimes become
translucent jelly CUBES (pseudo-3D top/side faces, pixel art, ref image of a
green ooze cube with things inside), squash on impact, a few colours. Digger
mob shelved (buggy at trees; should become a worm later) - stop spawning it.
Learner answers: smart cursor = hold Shift, hovered tile picks the belt tool
(block -> pickaxe, tree -> axe; walls/hammers later), release returns to the
hotbar item (round 2). Cubes green 1.5 / blue 2 / red 2.5 tiles. Things inside
cubes: bones by default (drop bone on death - bone = early crafting material
for starter weapons), rarer ores/items. Numbers: 5 hearts, sword 1 heart,
green 2 hits, red 5 hits, respawn 3 s.
Left click uses the held item (confirmed).
Implemented round 1: items.rs (Item, Inventory = 10-slot hotbar, starter kit),
icons.rs (16-px icon grid band at atlas y 1360: 3 cube sizes, sword, icons,
heart, slot, digits; small sprites bake their own rim), combat.rs (swept-arc
sword hits once per swing id, knockback+stun, popups, contact damage, bursting
into Drop entities, drop magnet + pickup), draw.rs (cubes GLASS with contents
drawn first under the same id, sword between body and front hand, drops,
popups, HUD in screen px via camera, scale = viewport_h/540). Shader flags
UNLIT/GLASS/ICON; id flag NO_OUTLINE (bit 31) skips the outline pass. Player:
health 100, hurt/knock/invulnerable blink/respawn 3 s at spawn, use_held,
sword_out. Mobs: green/blue/red cube kinds (40/60/100 hp, 10/15/25 dmg),
squash spring + stretch (area kept), contents 80% bones 1-2, digger never
spawned. Input: left held = use, right = mine (ponytail until smart cursor),
1-0 / wheel select.
Stage: implemented, 176 tests pass; awaiting learner review in-game (not committed)
Learner review (round 1): looks good, colours great. Cube 3D faces clash with
the flat world -> make cubes plain 2D. Too many cubes. Too easy: cubes should
leap like Terraria slimes (long calculated leaps, catching you when you jump
over), and holding the use button while walking keeps them off. Weapon reach
should come from weapon size. Swing must be smooth, not stepped.
Learner answers: flat cubes yes; spawn count - not sure, must not overwhelm a
new player (cubes are the only early mobs); leap AI yes; KEEP autoswing,
weapons differ by use time; Terraria-style smooth wide swing; reach from
sprite size; damage higher and by colour.
Implemented round 1b: flat cube painter; Spawner cap 4 / 8 s / 10 s grace;
leaper brain (pause 0.6-1.2 s with 0.3 s windup squash, launch aimed at
player pos + vel * flight time, clamped to kind.speed 6/7/8, every 3rd leap
x1.4 up x1.5 reach, idle hops beyond 30 tiles, no air steering); contact
damage 15/25/40; Weapon stats in items.rs (STARTER_SWORD 20 dmg, 0.4 s,
knockback 4.5) with reach measured from the painted sprite (icons::reach);
swing = continuous eased arc 3.75 -> 0.35 rad, 12 painted frames,
character::swing_hand gives the exact grip, combat sweeps the arc in small
steps. Autoswing kept.
Stage: done, committed (learner asked to commit after round 1b)

## Canopy dash fix
Black dashes above canopies: nearest-sampled mask UV at an exact pixel-centre
edge read the neighbouring atlas cell's row. Fixed by clamping mask lookups
inside their cell (shader.wgsl shade()). 180 tests pass.
Stage: implemented, learner confirmed it looks better (not committed).

## Smooth running: steps and swing-while-running
Learner asks: feet stop when swinging while running; run anim flickers to
fall on 1-tile drops; step-up too slow (STEP_DELAY 0.11 s blocks and zeroes
x speed). Wants Terraria feel: no momentum loss up or down 1-tile steps,
fast enough for future speed accessories (Hermes boots).
Decided: no step-up delay (Terraria-like); snap down 1-tile drops while walking
(glide sprite via negative step_lag); legs frame follows movement, upper body
follows action (Frame.legs + upper_shift from character::drop); moonwalk always.
Stage: implemented, 182 tests pass; awaiting in-game review (not committed).

## Formatting
Learner chose rustfmt, max_width 120 (rustfmt.toml). Committed separately as
83032cc (format only). Run cargo fmt before each commit.
