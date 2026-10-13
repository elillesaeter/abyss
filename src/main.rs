mod atlas;
mod autotile;
mod backdrop;
mod camera;
mod character;
mod combat;
mod draw;
mod entity;
mod icons;
mod items;
mod light;
mod mining;
mod mob;
mod noise;
mod physics;
mod player;
mod render;
mod shader_check;
mod sky;
mod tiles;
mod trees;
mod ui;
mod world;
mod worldgen;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use camera::Camera;
use entity::Entities;
use light::LightMap;
use mob::Spawner;
use noise::NoiseLog;
use player::{Intent, Player};
use render::Renderer;
use tiles::{AIR, DECOR_GRASS, DIRT, STONE};
use world::{WORLD_HEIGHT, WORLD_WIDTH, World};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = flag(&args, "--seed").and_then(|v| v.parse().ok()).unwrap_or(1337);

    if args.iter().any(|a| a == "--gen-atlas") {
        match atlas::generate_to_file() {
            Ok(()) => println!("wrote {}", atlas::PATH),
            Err(e) => eprintln!("could not write {}: {e}", atlas::PATH),
        }
        return;
    }

    let mut world = World::new(WORLD_WIDTH, WORLD_HEIGHT);
    let started = Instant::now();
    worldgen::generate(&mut world, seed);
    println!(
        "generated {}x{} tiles ({} chunks) with seed {seed} in {:?}",
        world.width(),
        world.height(),
        world.width_chunks() * world.height_chunks(),
        started.elapsed(),
    );

    if args.iter().any(|a| a == "--dump") {
        let x = flag(&args, "--x").and_then(|v| v.parse().ok()).unwrap_or(0);
        let w = flag(&args, "--w").and_then(|v| v.parse().ok()).unwrap_or(150);
        let h = flag(&args, "--h").and_then(|v| v.parse().ok()).unwrap_or(40);
        dump(&world, seed, x, w, h);
        return;
    }

    // `--time 19.5` starts at that hour; `--moon 0..7` picks the night's phase
    // (4 is full, 0 new), by starting that many days after the first.
    let hour = flag(&args, "--time").and_then(|v| v.parse().ok()).unwrap_or(8.0);
    let moon: u32 = flag(&args, "--moon").and_then(|v| v.parse().ok()).unwrap_or(4);
    world.sky = sky::Sky::at((moon + 4) % 8, hour);

    let frames: Option<u64> = flag(&args, "--frames").and_then(|v| v.parse().ok());
    let zoom = flag(&args, "--zoom").and_then(|v| v.parse().ok());

    if let Some(path) = flag(&args, "--screenshot") {
        let width = flag(&args, "--width").and_then(|v| v.parse().ok()).unwrap_or(1280);
        let height = flag(&args, "--height").and_then(|v| v.parse().ok()).unwrap_or(720);
        // `--inventory` captures with the inventory open and dirt on the cursor.
        let open = args.iter().any(|a| a == "--inventory");
        screenshot(world, seed, path, width, height, zoom, open);
        return;
    }
    println!(
        "A/D walks, W or Space jumps, left click uses the held item, hold Shift to \
         mine, chop or place a torch, 1-0 or the wheel picks, -/= zooms, hold T to \
         speed time up, Cmd+Q quits"
    );

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(world, seed, frames, zoom);
    event_loop.run_app(&mut app).expect("event loop");
}

/// Holding the right button on a slot takes one, then another after
/// `TAKE_FIRST`, then one every `TAKE_EVERY` seconds.
const TAKE_FIRST: f32 = 0.3;
const TAKE_EVERY: f32 = 0.15;

/// How much faster time runs while T is held: a whole day in 24 seconds.
const TIME_WARP: f32 = 60.0;

struct App {
    world: World,
    player: Player,
    entities: Entities,
    spawner: Spawner,
    noise: NoiseLog,
    light: LightMap,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    camera: Camera,
    held: HashSet<KeyCode>,
    cursor: [f32; 2],
    /// Held down, the left button keeps using the held item.
    using_held: bool,
    /// What the current hold uses instead: the cursor's item, or the smart
    /// cursor's pick when Shift was down at the press.
    latched: Option<items::Item>,
    inventory_open: bool,
    /// The right button held over the inventory, taking one at a time, and
    /// how long until the next.
    taking: bool,
    take_again: f32,
    viewport: (u32, u32),
    popups: Vec<combat::Popup>,
    /// `--zoom` from the command line, applied instead of fitting the default view.
    zoom_override: Option<usize>,
    last_frame: Instant,
    /// Frames to *present* before exiting; `None` runs until closed. Counting
    /// presented frames rather than redraw attempts matters because a window that
    /// is still appearing reports an unusable surface, and those frames don't count.
    frames_wanted: Option<u64>,
    started: Instant,
}

impl App {
    fn new(world: World, seed: u64, frames: Option<u64>, zoom: Option<usize>) -> App {
        let spawn_x = world.width() / 2;
        // Spawn clear of the ground so the first frame resolves by falling.
        let spawn_y = worldgen::surface_y(world.height(), seed, spawn_x) as f32 - 4.0;
        let spawn = [spawn_x as f32 + 0.5, spawn_y];
        let camera = Camera::new(spawn, (1280, 720));
        App {
            world,
            player: Player::new(spawn),
            entities: Entities::new(),
            spawner: Spawner::default(),
            noise: NoiseLog::default(),
            light: LightMap::default(),
            window: None,
            renderer: None,
            camera,
            zoom_override: zoom,
            held: HashSet::new(),
            cursor: [0.0, 0.0],
            using_held: false,
            latched: None,
            inventory_open: false,
            taking: false,
            take_again: 0.0,
            viewport: (1280, 720),
            popups: Vec::new(),
            last_frame: Instant::now(),
            frames_wanted: frames,
            started: Instant::now(),
        }
    }

    /// The left button went down: on the inventory it moves things about
    /// (Shift-click quick-moves); anywhere else it starts using an item, the
    /// one on the cursor if there is one.
    fn left_press(&mut self) {
        if self.inventory_open {
            let layout = ui::layout(&self.player.inventory, self.viewport);
            if let Some(slot) = layout.slot_at(self.cursor) {
                if self.smart() {
                    self.player.inventory.quick_move(slot);
                } else {
                    self.player.inventory.click(slot);
                }
                return;
            }
            if layout.covers(self.cursor) {
                return;
            }
        }
        // Used each frame while held; see RedrawRequested. The item is latched
        // for the whole hold, Shift or no Shift, wherever the pointer goes.
        self.using_held = true;
        if let Some(c) = self.player.inventory.cursor {
            self.latched = Some(c.item);
        } else if self.smart() {
            let at = self.camera.px_to_tile(self.cursor);
            let target = [at[0].floor() as i32, at[1].floor() as i32];
            self.latched = self
                .player
                .smart_item(&self.world, target)
                .or(self.player.inventory.held());
        }
    }

    /// The right button went down: on the inventory it takes one; anywhere
    /// else it throws what's on the cursor.
    fn right_press(&mut self) {
        if self.inventory_open {
            let layout = ui::layout(&self.player.inventory, self.viewport);
            if let Some(slot) = layout.slot_at(self.cursor) {
                self.player.inventory.take_one(slot);
                return;
            }
            if layout.covers(self.cursor) {
                return;
            }
        }
        if let Some(stack) = self.player.inventory.cursor.take() {
            combat::throw(&mut self.entities, &self.player, stack);
        }
    }

    fn held_any(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|k| self.held.contains(k))
    }

    /// Shift held: the smart cursor picks the tool for the hovered tile.
    fn smart(&self) -> bool {
        self.held_any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight])
    }

    /// A tool broke the tile at `tile`: a felled trunk brings its tree down,
    /// anything else drops its item, and the noise draws mobs.
    fn broke(&mut self, tile: [i32; 2], id: tiles::TileId) {
        let centre = [tile[0] as f32 + 0.5, tile[1] as f32 + 0.5];
        if id == tiles::TRUNK {
            trees::after_cut(&mut self.world, tile[0], tile[1], self.player.pos[0]);
        } else if let Some(item) = mining::drop_for(id) {
            combat::spawn_drop(&mut self.entities, &self.world, centre, item, 1);
        }
        self.noise.record(centre);
    }

    fn intent(&self) -> Intent {
        let left = self.held.contains(&KeyCode::KeyA);
        let right = self.held.contains(&KeyCode::KeyD);
        Intent {
            walk: (right as i32 - left as i32) as f32,
            jump: self.held.contains(&KeyCode::KeyW) || self.held.contains(&KeyCode::Space),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("abyss")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let size = window.inner_size();
        self.viewport = (size.width, size.height);
        self.camera.resize(size.width, size.height);
        match self.zoom_override {
            Some(step) => self.camera.set_zoom(step),
            None => self
                .camera
                .fit_view(camera::DEFAULT_VIEW_TILES.0, camera::DEFAULT_VIEW_TILES.1),
        }
        let (tw, th) = self.camera.view_tiles();
        println!(
            "window {}x{} physical: {} px/tile framing {tw:.0}x{th:.0} tiles",
            size.width,
            size.height,
            self.camera.pixels_per_tile(),
        );
        self.camera
            .follow(self.player.pos, self.world.width(), self.world.height());
        self.renderer = Some(Renderer::new(window.clone(), &self.world));
        self.window = Some(window);
        self.last_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.viewport = (size.width, size.height);
                self.camera.resize(size.width, size.height);
                self.camera.clamp_to_world(self.world.width(), self.world.height());
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = [position.x as f32, position.y as f32];
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state,
                ..
            } => {
                self.using_held = false;
                self.latched = None;
                if state == ElementState::Pressed {
                    self.left_press();
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state,
                ..
            } => {
                self.taking = state == ElementState::Pressed;
                self.take_again = TAKE_FIRST;
                if self.taking {
                    self.right_press();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if steps.abs() >= 1.0 {
                    let selected = self.player.inventory.selected as i32;
                    self.player.inventory.select(selected - steps.signum() as i32);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } => match (code, state) {
                (KeyCode::Escape, ElementState::Pressed) => {
                    self.inventory_open = !self.inventory_open;
                    // Whatever is on the cursor goes back in, or at your feet.
                    if !self.inventory_open
                        && let Some(left) = self.player.inventory.stow_cursor()
                    {
                        combat::throw(&mut self.entities, &self.player, left);
                    }
                }
                (KeyCode::KeyQ, ElementState::Pressed) if self.held_any(&[KeyCode::SuperLeft, KeyCode::SuperRight]) => {
                    event_loop.exit()
                }
                (KeyCode::Minus, ElementState::Pressed) => {
                    self.camera.zoom_out();
                    self.camera.clamp_to_world(self.world.width(), self.world.height());
                }
                (KeyCode::Equal, ElementState::Pressed) => {
                    self.camera.zoom_in();
                    self.camera.clamp_to_world(self.world.width(), self.world.height());
                }
                (code, ElementState::Pressed) => {
                    if let Some(slot) = hotbar_key(code) {
                        self.player.inventory.select(slot);
                    }
                    self.held.insert(code);
                }
                (code, ElementState::Released) => {
                    self.held.remove(&code);
                }
            },
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                let intent = self.intent();
                self.noise.advance(dt);
                for (pos, count) in trees::update(&mut self.world, dt) {
                    combat::spawn_drop(&mut self.entities, &self.world, pos, items::Item::Wood, count);
                }
                self.world.wear.update(dt);
                let speed = if self.held.contains(&KeyCode::KeyT) {
                    TIME_WARP
                } else {
                    1.0
                };
                self.world.sky.advance(dt * speed);
                if self.taking && self.inventory_open {
                    self.take_again -= dt;
                    if self.take_again <= 0.0 {
                        self.take_again = TAKE_EVERY;
                        if let Some(slot) = ui::layout(&self.player.inventory, self.viewport).slot_at(self.cursor) {
                            self.player.inventory.take_one(slot);
                        }
                    }
                }
                if self.using_held {
                    let at = self.camera.px_to_tile(self.cursor);
                    if let Some((tile, id)) = self.player.use_held(&mut self.world, at, self.latched) {
                        self.broke(tile, id);
                    }
                }
                self.player.update(&self.world, intent, dt);
                let view = self.camera.visible_tiles(self.world.width(), self.world.height());
                self.spawner
                    .tick(&mut self.entities, &self.world, view, self.player.pos, dt);
                mob::update(
                    &mut self.entities,
                    &mut self.world,
                    self.player.pos,
                    self.player.vel,
                    &self.noise,
                    dt,
                );
                combat::update(&self.world, &mut self.player, &mut self.entities, &mut self.popups, dt);
                self.camera
                    .follow_smoothed(self.player.visual_pos(), dt, self.world.width(), self.world.height());
                // Light last: it is computed for wherever the camera ended up.
                let lit = self.camera.visible_tiles(self.world.width(), self.world.height());
                let torch = combat::held_torch(&self.player);
                self.light.set_lamps(torch.map(|p| {
                    (
                        [p[0].floor() as i32, p[1].floor() as i32],
                        tiles::tile_type(tiles::TORCH).light,
                    )
                }));
                self.light.update(&self.world, lit, &self.world.sky.lighting());
                let hovered = self.camera.px_to_tile(self.cursor);
                let picked = self
                    .player
                    .smart_item(&self.world, [hovered[0].floor() as i32, hovered[1].floor() as i32]);
                let hud = draw::Hud {
                    smart: picked,
                    shift: self.smart(),
                    open: self.inventory_open,
                    pointer: self.cursor,
                };
                if let Some(renderer) = &mut self.renderer {
                    renderer.render(
                        &self.world,
                        &self.camera,
                        &self.player,
                        &self.entities,
                        &self.popups,
                        &self.light,
                        &hud,
                    );
                    if let Some(wanted) = self.frames_wanted {
                        let timed_out = self.started.elapsed() > Duration::from_secs(30);
                        if renderer.presented_frames() >= wanted || timed_out {
                            if timed_out {
                                eprintln!("timed out before presenting {wanted} frames");
                            }
                            println!(
                                "presented {} frames; last drew {} instances at {} px/tile; \
                                 assembly {:?} cached, peak {:?} rebuilding {} chunks; \
                                 {} mobs; player at [{:.2}, {:.2}] grounded={}",
                                renderer.presented_frames(),
                                renderer.instance_count(),
                                self.camera.pixels_per_tile(),
                                renderer.assembly_time(),
                                renderer.peak_assembly().0,
                                renderer.peak_assembly().1,
                                mob::count(&self.entities),
                                self.player.pos[0],
                                self.player.pos[1],
                                self.player.grounded,
                            );
                            event_loop.exit();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Number keys 1-9 then 0 pick hotbar slots left to right.
fn hotbar_key(code: KeyCode) -> Option<i32> {
    use KeyCode::*;
    [
        Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0,
    ]
    .iter()
    .position(|&k| k == code)
    .map(|i| i as i32)
}

/// Renders one frame without a window and writes it to a PNG. Needed because a
/// window launched from a terminal may never become visible, which makes the
/// windowed path unverifiable, and useful for eyeballing changes.
fn screenshot(mut world: World, seed: u64, path: &str, width: u32, height: u32, zoom: Option<usize>, open: bool) {
    let spawn_x = world.width() / 2;
    let spawn_y = worldgen::surface_y(world.height(), seed, spawn_x) as f32 - 4.0;
    let mut player = Player::new([spawn_x as f32 + 0.5, spawn_y]);
    // Let the player settle onto the ground so the capture shows a resting pose.
    for _ in 0..180 {
        player.update(&world, Intent::default(), 1.0 / 60.0);
    }

    let mut camera = Camera::new(player.pos, (width, height));
    match zoom {
        Some(step) => camera.set_zoom(step),
        None => camera.fit_view(camera::DEFAULT_VIEW_TILES.0, camera::DEFAULT_VIEW_TILES.1),
    }
    camera.follow(player.pos, world.width(), world.height());

    // A few mobs either side of the player, settled onto the ground, so a capture
    // shows them placed in the world rather than mid-fall.
    let mut entities = Entities::new();
    let mut log = NoiseLog::default();
    for (offset, kind) in [(-12.0, mob::GREEN_CUBE), (-7.0, mob::BLUE_CUBE), (10.0, mob::RED_CUBE)] {
        let x = player.pos[0] + offset;
        let surface = worldgen::surface_y(world.height(), seed, x as i32) as f32;
        mob::spawn(&mut entities, kind, [x, surface - 3.0], kind as u64);
    }
    for _ in 0..40 {
        mob::update(&mut entities, &mut world, player.pos, player.vel, &log, 1.0 / 60.0);
        log.advance(1.0 / 60.0);
    }
    // Mid-swing, with a damage number in the air, so a capture shows combat.
    player.use_held(&mut world, [player.pos[0] + 5.0, player.pos[1]], None);
    for _ in 0..8 {
        player.update(&world, Intent::default(), 1.0 / 60.0);
    }
    let popups = [combat::Popup {
        pos: [player.pos[0] + 3.0, player.pos[1] - 2.5],
        value: 20,
        age: 0.2,
        hurt_player: false,
    }];

    // A tunnel into the hillside with a torch in it, so a capture shows what the
    // lighting actually does: daylight outside, darkness inside, warm light around
    // a source. Capture-only scene dressing.
    // The terrain descends to the right, so tunnelling left at the player's foot
    // level runs into the hillside rather than out into open air.
    let floor = player.pos[1] as i32 + 2;
    let mouth = player.pos[0] as i32 - 1;
    for step in 0..20 {
        for y in floor - 3..floor {
            world.mine_for_capture(mouth - step, y);
        }
    }
    world.place(mouth - 13, floor - 1, tiles::TORCH);
    // One on the tunnel's end wall and one in the open air, to show how each
    // sits.
    world.place(mouth - 19, floor - 2, tiles::TORCH);
    world.place(player.pos[0] as i32 - 3, player.pos[1] as i32 - 3, tiles::TORCH);
    // A shaft straight down from the surface, to show the backdrop fading with depth.
    let shaft = player.pos[0] as i32 + 7;
    let top = worldgen::surface_y(world.height(), seed, shaft);
    for y in top..top + 26 {
        world.mine_for_capture(shaft, y);
        world.mine_for_capture(shaft + 1, y);
    }
    // A notch mined into the surface beside the player — the case that looked like
    // it was in shadow.
    for x in player.pos[0] as i32 - 7..player.pos[0] as i32 - 3 {
        let top = worldgen::surface_y(world.height(), seed, x);
        for y in top..top + 3 {
            world.mine_for_capture(x, y);
        }
    }

    // Cracks in the ground beside the player, a stage of each.
    for (dx, hits) in [(2, 1), (3, 2)] {
        let x = player.pos[0] as i32 + dx;
        let y = worldgen::surface_y(world.height(), seed, x);
        for _ in 0..hits {
            mining::hit(&mut world, [x, y], items::ToolKind::Pickaxe, 35);
        }
    }

    let mut light = LightMap::default();
    let mut renderer = Renderer::offscreen(&world, width, height);
    let lit = camera.visible_tiles(world.width(), world.height());
    let started = Instant::now();
    light.update(&world, lit, &world.sky.lighting());
    let light_time = started.elapsed();
    if open {
        player.inventory.click(items::Slot::Hotbar(1));
    }
    let hud = draw::Hud {
        smart: Some(items::Item::Pickaxe),
        open,
        pointer: [width as f32 * 0.5, height as f32 * 0.5],
        ..draw::Hud::default()
    };
    renderer.render(&world, &camera, &player, &entities, &popups, &light, &hud);
    // A second frame, after mining, exercises cache invalidation in the capture too.
    let feet = [player.pos[0] as i32, (player.pos[1] + player::HALF[1]) as i32 + 1];
    world.mine_for_capture(feet[0], feet[1]);
    light.update(&world, lit, &world.sky.lighting());
    renderer.render(&world, &camera, &player, &entities, &popups, &light, &hud);

    match renderer.save_capture(path) {
        Ok(()) => println!(
            "wrote {path} ({width}x{height} at {} px/tile, framing {:.0}x{:.0} tiles); {} mobs; {} instances ({} overlays), assembly {:?}, peak {:?} rebuilding {} chunks; light {:?} over {}x{} tiles",
            camera.pixels_per_tile(),
            camera.view_tiles().0,
            camera.view_tiles().1,
            mob::count(&entities),
            renderer.instance_count(),
            renderer.overlay_count(),
            renderer.assembly_time(),
            renderer.peak_assembly().0,
            renderer.peak_assembly().1,
            light_time,
            light.size()[0],
            light.size()[1],
        ),
        Err(e) => eprintln!("could not write {path}: {e}"),
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

/// Prints a window of the world as text, vertically centred on the terrain.
fn dump(world: &World, seed: u64, x0: i32, width: i32, height: i32) {
    let highest = (x0..(x0 + width).min(world.width()))
        .map(|x| worldgen::surface_y(world.height(), seed, x))
        .min()
        .unwrap_or(0);
    let y0 = (highest - 6).max(0);

    println!(
        "\nx {x0}..{}, y {y0}..{}  (' ' air, '\"' grass, '.' dirt, '#' stone, '|' trunk)\n",
        x0 + width,
        y0 + height
    );
    for y in y0..(y0 + height).min(world.height()) {
        let row: String = (x0..(x0 + width).min(world.width()))
            .map(|x| match world.get(x, y) {
                AIR => ' ',
                DIRT if world.decor(x, y) & DECOR_GRASS != 0 => '"',
                DIRT => '.',
                STONE => '#',
                tiles::TRUNK => '|',
                _ => '?',
            })
            .collect();
        println!("{row}");
    }
}
