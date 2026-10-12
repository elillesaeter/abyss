//! View onto the world, in tile units.
//!
//! Zoom is an integer number of physical pixels per tile and the camera centre is
//! snapped so tile edges land on whole physical pixels. Without both, quad edges
//! fall between pixels and crawl visibly while panning.

/// How quickly the camera closes the gap to its target, per second. Higher is
/// stiffer; this reaches roughly 90% of the distance in a quarter of a second.
const FOLLOW_RATE: f32 = 9.0;

/// Physical pixels per tile. Integer steps only, because a fractional tile size
/// puts quad edges between pixels and makes them crawl while panning.
pub const ZOOM_STEPS: [u32; 10] = [4, 6, 8, 10, 12, 16, 20, 24, 28, 32];
const DEFAULT_ZOOM: usize = 5;

/// How much world the camera should show by default, in tiles. This is the framing
/// that matters to play, and it is what stays constant across displays — pixels per
/// tile is derived from it, not the other way round.
pub const DEFAULT_VIEW_TILES: (f32, f32) = (100.0, 58.0);

/// Half-open tile rectangle: `x0..x1`, `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl TileRect {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }

    pub fn tiles(&self) -> i64 {
        self.width() as i64 * self.height() as i64
    }
}

/// `ndc = tile_position * scale + offset`. A 2D orthographic view needs nothing
/// more than this, so the uniform is 16 bytes instead of a 4x4 matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

pub struct Camera {
    /// View centre in tile units.
    pub pos: [f32; 2],
    zoom: usize,
    /// Viewport size in physical pixels.
    viewport: (u32, u32),
    /// The framing a resize fits again, until the zoom is set by hand.
    fit: Option<(f32, f32)>,
}

impl Camera {
    pub fn new(pos: [f32; 2], viewport: (u32, u32)) -> Camera {
        Camera {
            pos,
            zoom: DEFAULT_ZOOM,
            viewport,
            fit: None,
        }
    }

    pub fn pixels_per_tile(&self) -> f32 {
        ZOOM_STEPS[self.zoom] as f32
    }

    /// Sets the zoom step directly, clamped. Used by the command line.
    pub fn set_zoom(&mut self, step: usize) {
        self.zoom = step.min(ZOOM_STEPS.len() - 1);
        self.fit = None;
    }

    /// Picks the zoom step that frames closest to the requested number of tiles on
    /// the current viewport, so the same framing is reached on a 2560x1440 monitor
    /// and on a Retina panel. Both axes must fit, so the tighter constraint wins;
    /// the result is the nearest integer step, which may differ by a few tiles.
    /// Resizes keep this framing, until the zoom is changed by hand.
    pub fn fit_view(&mut self, tiles_wide: f32, tiles_tall: f32) {
        self.fit = Some((tiles_wide, tiles_tall));
        let ideal = (self.viewport.0 as f32 / tiles_wide).min(self.viewport.1 as f32 / tiles_tall);
        self.zoom = ZOOM_STEPS
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let (da, db) = ((**a as f32 - ideal).abs(), (**b as f32 - ideal).abs());
                da.partial_cmp(&db).expect("zoom steps are finite")
            })
            .map(|(i, _)| i)
            .unwrap_or(DEFAULT_ZOOM);
    }

    /// Tiles currently framed, as (width, height).
    pub fn view_tiles(&self) -> (f32, f32) {
        let ppt = self.pixels_per_tile();
        (self.viewport.0 as f32 / ppt, self.viewport.1 as f32 / ppt)
    }

    pub fn zoom_in(&mut self) {
        self.set_zoom(self.zoom + 1);
    }

    pub fn zoom_out(&mut self) {
        self.set_zoom(self.zoom.saturating_sub(1));
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.viewport = (width.max(1), height.max(1));
        if let Some((w, h)) = self.fit {
            self.fit_view(w, h);
        }
    }

    /// Integer pixel offset of the view centre within the viewport. Uses integer
    /// division so an odd viewport doesn't put the centre on a half pixel.
    fn center_px(&self) -> (i32, i32) {
        ((self.viewport.0 / 2) as i32, (self.viewport.1 / 2) as i32)
    }

    /// Camera centre rounded to a whole physical pixel.
    pub fn snapped_pos(&self) -> [f32; 2] {
        let ppt = self.pixels_per_tile();
        [(self.pos[0] * ppt).round() / ppt, (self.pos[1] * ppt).round() / ppt]
    }

    /// Screen pixel of a tile corner, for verifying pixel alignment.
    pub fn tile_to_px(&self, tile: [f32; 2]) -> [f32; 2] {
        let ppt = self.pixels_per_tile();
        let cam = self.snapped_pos();
        let (cx, cy) = self.center_px();
        [
            (tile[0] - cam[0]) * ppt + cx as f32,
            (tile[1] - cam[1]) * ppt + cy as f32,
        ]
    }

    /// Inverse of `tile_to_px`, for turning a cursor position into a tile.
    pub fn px_to_tile(&self, px: [f32; 2]) -> [f32; 2] {
        let ppt = self.pixels_per_tile();
        let cam = self.snapped_pos();
        let (cx, cy) = self.center_px();
        [(px[0] - cx as f32) / ppt + cam[0], (px[1] - cy as f32) / ppt + cam[1]]
    }

    /// Centres on a target immediately. Used when placing the camera, not per frame.
    pub fn follow(&mut self, target: [f32; 2], world_width: i32, world_height: i32) {
        self.pos = target;
        self.clamp_to_world(world_width, world_height);
    }

    /// Eases toward a target by a fixed fraction of the remaining distance per
    /// second, so the camera lags briefly instead of being welded to the player.
    /// Framerate independent, and it cannot overshoot: the step is always a
    /// fraction of what is left.
    pub fn follow_smoothed(&mut self, target: [f32; 2], dt: f32, world_width: i32, world_height: i32) {
        let t = 1.0 - (-FOLLOW_RATE * dt).exp();
        for axis in 0..2 {
            self.pos[axis] += (target[axis] - self.pos[axis]) * t;
        }
        self.clamp_to_world(world_width, world_height);
    }

    pub fn view_transform(&self) -> ViewTransform {
        let ppt = self.pixels_per_tile();
        let (w, h) = (self.viewport.0 as f32, self.viewport.1 as f32);
        let cam = self.snapped_pos();
        let (cx, cy) = self.center_px();
        // px = (tile - cam) * ppt + centre, then ndc_x = px/w*2 - 1 and
        // ndc_y = 1 - px/h*2 because world y grows downward.
        ViewTransform {
            scale: [2.0 * ppt / w, -2.0 * ppt / h],
            offset: [
                2.0 * (cx as f32 - cam[0] * ppt) / w - 1.0,
                1.0 - 2.0 * (cy as f32 - cam[1] * ppt) / h,
            ],
        }
    }

    /// Tiles touching the viewport, clamped to the world, with a one-tile margin
    /// so partially visible edge tiles are never dropped.
    pub fn visible_tiles(&self, world_width: i32, world_height: i32) -> TileRect {
        let ppt = self.pixels_per_tile();
        let cam = self.snapped_pos();
        let (cx, cy) = self.center_px();
        let x0 = cam[0] - cx as f32 / ppt;
        let y0 = cam[1] - cy as f32 / ppt;
        let x1 = cam[0] + (self.viewport.0 as i32 - cx) as f32 / ppt;
        let y1 = cam[1] + (self.viewport.1 as i32 - cy) as f32 / ppt;
        TileRect {
            x0: (x0.floor() as i32 - 1).clamp(0, world_width),
            y0: (y0.floor() as i32 - 1).clamp(0, world_height),
            x1: (x1.ceil() as i32 + 1).clamp(0, world_width),
            y1: (y1.ceil() as i32 + 1).clamp(0, world_height),
        }
    }

    /// Pans by a tile-unit delta, keeping the view inside the world. When the world
    /// is smaller than the viewport on an axis, the camera locks to its centre.
    pub fn pan(&mut self, dx: f32, dy: f32, world_width: i32, world_height: i32) {
        self.pos[0] += dx;
        self.pos[1] += dy;
        self.clamp_to_world(world_width, world_height);
    }

    pub fn clamp_to_world(&mut self, world_width: i32, world_height: i32) {
        let ppt = self.pixels_per_tile();
        let half = [self.viewport.0 as f32 / 2.0 / ppt, self.viewport.1 as f32 / 2.0 / ppt];
        let world = [world_width as f32, world_height as f32];
        for axis in 0..2 {
            self.pos[axis] = if world[axis] <= half[axis] * 2.0 {
                world[axis] / 2.0
            } else {
                self.pos[axis].clamp(half[axis], world[axis] - half[axis])
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORTS: [(u32, u32); 3] = [(1920, 1080), (1512, 945), (801, 603)];

    #[test]
    fn visible_rect_covers_the_viewport_without_gross_overdraw() {
        for viewport in VIEWPORTS {
            for zoom in 0..ZOOM_STEPS.len() {
                let mut cam = Camera::new([1000.0, 300.0], viewport);
                cam.zoom = zoom;
                let rect = cam.visible_tiles(100_000, 100_000);
                let ppt = cam.pixels_per_tile();
                let needed = (viewport.0 as f32 / ppt, viewport.1 as f32 / ppt);
                assert!(
                    rect.width() as f32 >= needed.0 && rect.height() as f32 >= needed.1,
                    "gap at zoom {ppt} viewport {viewport:?}: {rect:?} covers less than {needed:?}"
                );
                assert!(
                    rect.width() as f32 <= needed.0 + 6.0 && rect.height() as f32 <= needed.1 + 6.0,
                    "overdraw at zoom {ppt} viewport {viewport:?}: {rect:?} for {needed:?}"
                );
            }
        }
    }

    #[test]
    fn visible_rect_is_clamped_to_the_world() {
        let cam = Camera::new([0.0, 0.0], (1920, 1080));
        let rect = cam.visible_tiles(2112, 608);
        assert_eq!((rect.x0, rect.y0), (0, 0));
        assert!(rect.x1 <= 2112 && rect.y1 <= 608);
    }

    #[test]
    fn tile_edges_land_on_whole_pixels_at_every_zoom() {
        for zoom in 0..ZOOM_STEPS.len() {
            let mut cam = Camera::new([1000.37, 299.81], (1512, 945));
            cam.zoom = zoom;
            for tile in [[0.0, 0.0], [1000.0, 300.0], [2111.0, 607.0]] {
                let px = cam.tile_to_px(tile);
                for v in px {
                    let off = (v - v.round()).abs();
                    assert!(off < 1e-3, "tile edge at {v} px is off the pixel grid by {off}");
                }
            }
        }
    }

    #[test]
    fn pixel_and_tile_conversions_round_trip() {
        for zoom in 0..ZOOM_STEPS.len() {
            let mut cam = Camera::new([1000.37, 299.81], (1512, 945));
            cam.zoom = zoom;
            for tile in [[0.0, 0.0], [1000.5, 300.25], [2111.0, 607.0]] {
                let back = cam.px_to_tile(cam.tile_to_px(tile));
                assert!(
                    (back[0] - tile[0]).abs() < 1e-3 && (back[1] - tile[1]).abs() < 1e-3,
                    "{tile:?} round-tripped to {back:?} at zoom {}",
                    cam.pixels_per_tile()
                );
            }
        }
    }

    #[test]
    fn snapping_is_idempotent() {
        let mut cam = Camera::new([1000.37, 299.81], (1512, 945));
        let once = cam.snapped_pos();
        cam.pos = once;
        assert_eq!(cam.snapped_pos(), once);
    }

    #[test]
    fn fit_view_reaches_the_same_framing_on_different_displays() {
        // A 2560x1440 monitor at scale 1, and a 1280x720 window on a Retina panel.
        for viewport in [(2560, 1440), (1280, 720)] {
            let mut cam = Camera::new([1000.0, 300.0], viewport);
            cam.fit_view(DEFAULT_VIEW_TILES.0, DEFAULT_VIEW_TILES.1);
            let (w, h) = cam.view_tiles();
            assert!(
                (w - DEFAULT_VIEW_TILES.0).abs() < 15.0 && (h - DEFAULT_VIEW_TILES.1).abs() < 10.0,
                "{viewport:?} framed {w:.0}x{h:.0} tiles, wanted {DEFAULT_VIEW_TILES:?}"
            );
        }
    }

    #[test]
    fn a_fitted_view_refits_on_resize_until_zoomed_by_hand() {
        let mut cam = Camera::new([1000.0, 300.0], (1280, 720));
        cam.fit_view(DEFAULT_VIEW_TILES.0, DEFAULT_VIEW_TILES.1);
        let windowed = cam.pixels_per_tile();
        // Going fullscreen keeps the framing, not the pixels per tile.
        cam.resize(2560, 1440);
        assert!(cam.pixels_per_tile() > windowed);
        assert!((cam.view_tiles().0 - DEFAULT_VIEW_TILES.0).abs() < 15.0);
        // Once zoomed by hand, the chosen zoom stays.
        cam.zoom_in();
        let chosen = cam.pixels_per_tile();
        cam.resize(1280, 720);
        assert_eq!(cam.pixels_per_tile(), chosen);
    }

    #[test]
    fn fit_view_keeps_both_axes_within_the_request() {
        // The tighter axis decides, so neither axis ends up showing far too little.
        let mut cam = Camera::new([1000.0, 300.0], (3456, 2234));
        cam.fit_view(120.0, 70.0);
        let (w, h) = cam.view_tiles();
        assert!(w >= 100.0 && h >= 60.0, "framed only {w:.0}x{h:.0} tiles");
        assert!(
            ZOOM_STEPS.contains(&(cam.pixels_per_tile() as u32)),
            "chose a non-integer step"
        );
    }

    #[test]
    fn zoom_clamps_at_both_ends() {
        let mut cam = Camera::new([0.0, 0.0], (1920, 1080));
        for _ in 0..20 {
            cam.zoom_in();
        }
        assert_eq!(cam.pixels_per_tile(), *ZOOM_STEPS.last().unwrap() as f32);
        for _ in 0..20 {
            cam.zoom_out();
        }
        assert_eq!(cam.pixels_per_tile(), ZOOM_STEPS[0] as f32);
    }

    #[test]
    fn smoothed_follow_converges_without_overshooting() {
        let mut cam = Camera::new([1000.0, 300.0], (1920, 1080));
        let target = [1040.0, 300.0];
        let mut previous = cam.pos[0];
        for _ in 0..60 {
            cam.follow_smoothed(target, 1.0 / 60.0, 100_000, 100_000);
            assert!(cam.pos[0] > previous, "should keep closing the gap");
            assert!(cam.pos[0] <= target[0], "overshot to {}", cam.pos[0]);
            previous = cam.pos[0];
        }
        assert!(
            (cam.pos[0] - target[0]).abs() < 0.5,
            "a second of following should land near the target, got {}",
            cam.pos[0]
        );
    }

    #[test]
    fn smoothed_follow_lags_at_first_and_is_framerate_independent() {
        // Well clear of the world edges, so clamping plays no part in the result.
        const START: f32 = 1000.0;
        const TARGET: f32 = 1100.0;
        let travelled = |dt: f32, steps: usize| {
            let mut cam = Camera::new([START, 300.0], (1920, 1080));
            for _ in 0..steps {
                cam.follow_smoothed([TARGET, 300.0], dt, 100_000, 100_000);
            }
            cam.pos[0] - START
        };
        let after_one_frame = travelled(1.0 / 60.0, 1);
        assert!(after_one_frame < 20.0, "camera should lag, moved {after_one_frame}");

        // Quarter of a second at 60 Hz and at 30 Hz must land in the same place.
        let (fast, slow) = (travelled(1.0 / 60.0, 15), travelled(1.0 / 30.0, 8));
        assert!((fast - slow).abs() < 2.0, "{fast} at 60 Hz vs {slow} at 30 Hz");
    }

    #[test]
    fn smoothed_follow_still_clamps_to_the_world() {
        let mut cam = Camera::new([1000.0, 300.0], (1920, 1080));
        for _ in 0..240 {
            cam.follow_smoothed([-5000.0, -5000.0], 1.0 / 60.0, 2112, 608);
        }
        let rect = cam.visible_tiles(2112, 608);
        assert!(rect.x0 >= 0 && rect.y0 >= 0, "escaped the world: {rect:?}");
    }

    #[test]
    fn panning_keeps_the_view_inside_the_world() {
        let mut cam = Camera::new([1000.0, 300.0], (1920, 1080));
        for (dx, dy) in [(-5000.0, -5000.0), (5000.0, 5000.0)] {
            cam.pan(dx, dy, 2112, 608);
            let rect = cam.visible_tiles(2112, 608);
            assert!(rect.x0 >= 0 && rect.x1 <= 2112, "escaped horizontally: {rect:?}");
            assert!(rect.y0 >= 0 && rect.y1 <= 608, "escaped vertically: {rect:?}");
        }
    }

    #[test]
    fn camera_centres_on_an_axis_smaller_than_the_viewport() {
        let mut cam = Camera::new([0.0, 0.0], (1920, 1080));
        cam.zoom_in();
        cam.zoom_in(); // 32 px/tile: 1080 px of viewport is 33.75 tiles
        cam.clamp_to_world(2112, 20);
        assert_eq!(cam.pos[1], 10.0, "a world shorter than the view should sit centred");
    }
}
