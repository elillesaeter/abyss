//! The player's look, painted by code at startup: a paper doll of parts, each in
//! greys only, drawn one over another and coloured by multiplying with the
//! wearer's `Appearance`. Every frame is painted from a `Pose` — where the hips,
//! knees, feet, elbows and hands are — so anything else worn (armour, later)
//! can be painted from the same poses and sits right in every frame.

use std::f32::consts::TAU;

use crate::atlas::{FRAME_COLS, FRAME_H, FRAME_W, SPRITE_BAND_Y, WIDTH};

/// Frames, all facing right; left is drawn mirrored.
pub mod frames {
    pub const IDLE: u16 = 0;
    pub const WALK: [u16; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    pub const RISE: u16 = 9;
    pub const FALL: u16 = 10;
    /// The arm swung from behind the head down to in front, sampled along the
    /// arc: mining, placing, swinging.
    pub const USE: [u16; 12] = [11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22];
    pub const COUNT: u16 = 23;
}

/// The parts, each with its own row of frames in the atlas and its own colour.
/// The arms are split so the far one goes behind the body and the near one in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    BackHand,
    BackSleeve,
    Skin,
    Trousers,
    Shoes,
    Shirt,
    EyeWhites,
    Eyes,
    Hair,
    FrontHand,
    FrontSleeve,
}

/// Back to front: the order parts are drawn in, and their rows in the atlas.
pub const PARTS: [Part; 11] = [
    Part::BackHand,
    Part::BackSleeve,
    Part::Skin,
    Part::Trousers,
    Part::Shoes,
    Part::Shirt,
    Part::EyeWhites,
    Part::Eyes,
    Part::Hair,
    Part::FrontHand,
    Part::FrontSleeve,
];

/// What a character looks like: one colour per part. Character creation fills
/// this in; until then everyone gets the default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub skin: [u8; 3],
    pub hair: [u8; 3],
    pub eyes: [u8; 3],
    pub shirt: [u8; 3],
    pub trousers: [u8; 3],
    pub shoes: [u8; 3],
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance {
            skin: [255, 212, 178],
            hair: [128, 78, 44],
            eyes: [62, 48, 40],
            shirt: [86, 128, 186],
            trousers: [112, 92, 72],
            shoes: [78, 60, 52],
        }
    }
}

impl Appearance {
    /// The colour a part's greys are multiplied by.
    pub fn tint(&self, part: Part) -> [u8; 4] {
        let [r, g, b] = match part {
            Part::BackHand | Part::FrontHand | Part::Skin => self.skin,
            Part::BackSleeve | Part::FrontSleeve | Part::Shirt => self.shirt,
            Part::Trousers => self.trousers,
            Part::Shoes => self.shoes,
            Part::Eyes => self.eyes,
            Part::EyeWhites => [255; 3],
            Part::Hair => self.hair,
        };
        [r, g, b, 255]
    }
}

/// Greys from shadow to highlight. The tint is the colour at about the fourth.
const SHADES: [u8; 5] = [84, 124, 164, 208, 246];
/// Toward the light, top left, matching the bark.
const LIGHT: [f32; 2] = [-0.6, -0.8];

/// Where the sole of a standing foot's ankle is, in frame pixels: the shoe
/// beneath it fills the frame's bottom rows.
const ANKLE_Y: f32 = FRAME_H as f32 - 4.0;
// Terraria-like proportions: a big head, a torso as wide as the head is tall,
// short stubby legs and thick arms.
const HIP: [f32; 2] = [20.0, 43.0];
const SHOULDER: [f32; 2] = [20.0, 31.0];
const THIGH: f32 = 8.5;
const SHIN: f32 = 8.5;
const UPPER_ARM: f32 = 5.0;
const FOREARM: f32 = 5.0;
const LEG_R: f32 = 4.0;
const SLEEVE_R: f32 = 2.8;
const HAND_R: f32 = 2.4;

/// A limb's angle from hanging straight down (positive swings it forward) and
/// how far its lower half turns from that: a knee bends back, an elbow forward.
#[derive(Clone, Copy)]
struct Limb {
    angle: f32,
    bend: f32,
}

const fn limb(angle: f32, bend: f32) -> Limb {
    Limb { angle, bend }
}

struct Pose {
    back_leg: Limb,
    front_leg: Limb,
    back_arm: Limb,
    front_arm: Limb,
}

fn pose(frame: u16) -> Pose {
    let (back_leg, front_leg, back_arm, front_arm) = match frame {
        frames::IDLE => (limb(-0.06, 0.0), limb(0.06, 0.0), limb(-0.08, 0.1), limb(0.08, 0.1)),
        f if frames::WALK.contains(&f) => {
            let phase = TAU * (f - frames::WALK[0]) as f32 / frames::WALK.len() as f32;
            let (s, c) = phase.sin_cos();
            // A leg bends its knee while it swings forward, and the arms swing
            // against the legs.
            (
                limb(-0.5 * s, 0.9 * (-c).max(0.0)),
                limb(0.5 * s, 0.9 * c.max(0.0)),
                limb(0.5 * s, 0.3),
                limb(-0.5 * s, 0.3),
            )
        }
        frames::RISE => (limb(-0.25, 0.5), limb(0.6, 1.2), limb(-0.6, 0.3), limb(2.3, 0.4)),
        // Flailing: arms up and out, one each way.
        frames::FALL => (limb(-0.3, 0.2), limb(0.3, 0.3), limb(-2.3, -0.2), limb(2.3, 0.2)),
        f => {
            return swing_pose(f.saturating_sub(frames::USE[0]) as f32 / (frames::USE.len() - 1) as f32);
        }
    };
    Pose {
        back_leg,
        front_leg,
        back_arm,
        front_arm,
    }
}

/// The swing's arc, as the front arm's angle from hanging down: from reaching
/// back over the head to low in front of the feet.
const SWING_FROM: f32 = 3.75;
const SWING_TO: f32 = 0.35;

/// How far along its arc a swing is, `t` of the way through its time: fast
/// out of the wind-up, slowing into the follow-through.
pub fn swing_arc(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(2)
}

/// The pose `arc` of the way along the swing, feet planted.
fn swing_pose(arc: f32) -> Pose {
    Pose {
        back_leg: limb(-0.2, 0.0),
        front_leg: limb(0.25, 0.1),
        back_arm: limb(-0.3, 0.2),
        front_arm: limb(SWING_FROM + (SWING_TO - SWING_FROM) * arc, 0.15),
    }
}

/// The painted frame nearest `arc` along the swing.
pub fn use_frame(arc: f32) -> u16 {
    frames::USE[(arc.clamp(0.0, 1.0) * (frames::USE.len() - 1) as f32).round() as usize]
}

/// Whole art pixels frame `frame` lowers the body by to stand on its feet.
pub fn drop(frame: u16) -> i32 {
    skeleton(&pose(frame)).drop
}

/// A pose turned into points in the frame, the body lowered until the lower
/// foot stands on the frame's ankle line.
struct Skeleton {
    /// Whole pixels the body is lowered by.
    drop: i32,
    /// Hip, knee, ankle: back leg then front.
    legs: [[[f32; 2]; 3]; 2],
    /// Shoulder, elbow, hand: back arm then front.
    arms: [[[f32; 2]; 3]; 2],
}

fn joint(from: [f32; 2], angle: f32, length: f32) -> [f32; 2] {
    [from[0] + length * angle.sin(), from[1] + length * angle.cos()]
}

fn skeleton(pose: &Pose) -> Skeleton {
    let leg = |hip: [f32; 2], l: Limb| {
        let knee = joint(hip, l.angle, THIGH);
        [hip, knee, joint(knee, l.angle - l.bend, SHIN)]
    };
    let arm = |shoulder: [f32; 2], l: Limb| {
        let elbow = joint(shoulder, l.angle, UPPER_ARM);
        [shoulder, elbow, joint(elbow, l.angle + l.bend, FOREARM)]
    };
    let mut legs = [
        leg([HIP[0] - 2.0, HIP[1]], pose.back_leg),
        leg([HIP[0] + 2.0, HIP[1]], pose.front_leg),
    ];
    let lowest = legs[0][2][1].max(legs[1][2][1]);
    let drop = (ANKLE_Y - lowest).round() as i32;
    for leg in &mut legs {
        for p in leg {
            p[1] += drop as f32;
        }
    }
    let down = |p: [f32; 2]| [p[0], p[1] + drop as f32];
    let arms = [
        arm(down([SHOULDER[0] - 1.0, SHOULDER[1]]), pose.back_arm),
        arm(down(SHOULDER), pose.front_arm),
    ];
    Skeleton { drop, legs, arms }
}

/// One part of one frame: a grey per pixel, 0 where nothing is painted.
struct Canvas([u8; (FRAME_W * FRAME_H) as usize]);

impl Canvas {
    fn new() -> Canvas {
        Canvas([0; (FRAME_W * FRAME_H) as usize])
    }

    fn set(&mut self, x: i32, y: i32, shade: i32) {
        if (0..FRAME_W as i32).contains(&x) && (0..FRAME_H as i32).contains(&y) {
            self.0[(y as u32 * FRAME_W + x as u32) as usize] = SHADES[shade.clamp(0, 4) as usize];
        }
    }

    /// A round-ended bar from `a` to `b`, shaded as a cylinder lit from the top
    /// left, with a dark rim on its shadowed side where it meets what's behind.
    /// `dark` lowers every shade, for limbs on the far side.
    fn bar(&mut self, a: [f32; 2], b: [f32; 2], r: f32, dark: i32) {
        let (x0, x1) = ((a[0].min(b[0]) - r).floor() as i32, (a[0].max(b[0]) + r).ceil() as i32);
        let (y0, y1) = ((a[1].min(b[1]) - r).floor() as i32, (a[1].max(b[1]) + r).ceil() as i32);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let len2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
                let n = [p[0] - a[0] - t * ab[0], p[1] - a[1] - t * ab[1]];
                let d = n[0].hypot(n[1]);
                if d > r {
                    continue;
                }
                let lit = (n[0] * LIGHT[0] + n[1] * LIGHT[1]) / r;
                let shade = if d > r - 0.9 && lit < 0.0 { 0 } else { lit_shade(lit) };
                self.set(x, y, shade - dark);
            }
        }
    }

    /// A box with clipped corners, shaded as a rounded block lit from the top
    /// left, rimmed along its shadowed bottom and right.
    fn block(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, dark: i32) {
        let (cx, cy) = ((x0 + x1) as f32 * 0.5, (y0 + y1) as f32 * 0.5);
        let (hw, hh) = ((x1 - x0) as f32 * 0.5 + 0.5, (y1 - y0) as f32 * 0.5 + 0.5);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x == x0 || x == x1) && (y == y0 || y == y1) {
                    continue;
                }
                let n = [(x as f32 - cx) / hw, (y as f32 - cy) / hh];
                let lit = n[0] * LIGHT[0] + n[1] * LIGHT[1];
                let shade = if x == x1 || y == y1 { 0 } else { lit_shade(lit) };
                self.set(x, y, shade - dark);
            }
        }
    }
}

fn lit_shade(lit: f32) -> i32 {
    match lit {
        l if l > 0.4 => 4,
        l if l > -0.1 => 3,
        l if l > -0.55 => 2,
        _ => 1,
    }
}

/// Boxes as [left, top, right, bottom], inclusive, before the body is lowered.
const HEAD: [i32; 4] = [13, 14, 28, 28];
const TORSO: [i32; 4] = [13, 29, 27, 42];

fn paint_part(part: Part, s: &Skeleton) -> Canvas {
    let mut c = Canvas::new();
    let d = s.drop;
    let hand = |c: &mut Canvas, arm: &[[f32; 2]; 3], dark| c.bar(arm[2], arm[2], HAND_R, dark);
    let sleeve = |c: &mut Canvas, arm: &[[f32; 2]; 3], dark| {
        let [shoulder, elbow, hand] = *arm;
        let wrist = [
            hand[0] + (elbow[0] - hand[0]) * 0.25,
            hand[1] + (elbow[1] - hand[1]) * 0.25,
        ];
        c.bar(shoulder, elbow, SLEEVE_R, dark);
        c.bar(elbow, wrist, SLEEVE_R - 0.2, dark);
    };
    match part {
        Part::BackHand => hand(&mut c, &s.arms[0], 1),
        Part::BackSleeve => sleeve(&mut c, &s.arms[0], 1),
        Part::FrontHand => hand(&mut c, &s.arms[1], 0),
        Part::FrontSleeve => sleeve(&mut c, &s.arms[1], 0),
        Part::Skin => c.block(HEAD[0], HEAD[1] + d, HEAD[2], HEAD[3] + d, 0),
        // A white column with the coloured iris in front of it, looking ahead.
        Part::EyeWhites => {
            for y in EYE_TOP..EYE_TOP + 3 {
                c.set(HEAD[2] - 4, y + d, 4);
            }
        }
        Part::Eyes => {
            for y in EYE_TOP..EYE_TOP + 3 {
                c.set(HEAD[2] - 3, y + d, 3);
            }
        }
        Part::Shirt => c.block(TORSO[0], TORSO[1] + d, TORSO[2], TORSO[3] + d, 0),
        Part::Trousers => {
            for (leg, dark) in s.legs.iter().zip([1, 0]) {
                c.bar(leg[0], leg[1], LEG_R, dark);
                c.bar(leg[1], leg[2], LEG_R - 0.4, dark);
            }
            c.block(TORSO[0] + 1, TORSO[3] - 2 + d, TORSO[2] - 1, HIP[1] as i32 + 1 + d, 0);
        }
        Part::Shoes => {
            for (leg, dark) in s.legs.iter().zip([1, 0]) {
                let (x, y) = (leg[2][0].round() as i32, leg[2][1].round() as i32);
                c.block(x - 3, y, x + 4, y + 3, dark);
            }
        }
        Part::Hair => hair(&mut c, d),
    }
    c
}

const EYE_TOP: i32 = HEAD[1] + 7;

/// Full short hair: a thick rounded cap over the crown, long at the back of the
/// head, and a fringe over the brow that stops above the eyes.
fn hair(c: &mut Canvas, d: i32) {
    let [x0, y0, x1, _] = HEAD;
    for x in x0 - 1..=x1 + 1 {
        let i = x - x0;
        let bottom = match i {
            _ if i <= 4 => y0 + 11,
            _ if i <= 7 => y0 + 5,
            _ if x >= x1 - 1 => y0 + 5,
            _ => y0 + 4,
        };
        // Rounded: the outer columns start lower.
        let top = match (x - x0 + 1).min(x1 + 1 - x) {
            0 => y0 + 2,
            1 => y0,
            2 => y0 - 1,
            _ => y0 - 2,
        };
        for y in top..=bottom {
            let mut shade = if y < y0 + 1 { 4 } else { 3 };
            // Strands: darker lines running down and back.
            if (x + (y - y0) / 2).rem_euclid(3) == 0 {
                shade -= 1;
            }
            if y == bottom || x == x0 - 1 || y == top && x > x1 - 2 {
                shade = 1;
            }
            c.set(x, y + d, shade);
        }
    }
}

/// The near hand `arc` of the way along a swing, and the way the forearm
/// points out of it: where a held weapon's grip sits and where it aims, exact
/// between painted frames. Frame pixels, facing right.
pub fn swing_hand(arc: f32) -> ([f32; 2], [f32; 2]) {
    let [_, elbow, hand] = skeleton(&swing_pose(arc)).arms[1];
    let d = [hand[0] - elbow[0], hand[1] - elbow[1]];
    let len = d[0].hypot(d[1]).max(1e-6);
    (hand, [d[0] / len, d[1] / len])
}

/// Paints every part of every frame into the atlas's sprite band.
pub fn paint(px: &mut [u8]) {
    for frame in 0..frames::COUNT {
        let s = skeleton(&pose(frame));
        for (row, &part) in PARTS.iter().enumerate() {
            let canvas = paint_part(part, &s);
            let (ox, oy) = cell(row, frame);
            for y in 0..FRAME_H {
                for x in 0..FRAME_W {
                    let g = canvas.0[(y * FRAME_W + x) as usize];
                    let i = (((oy + y) * WIDTH + ox + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[g, g, g, if g > 0 { 255 } else { 0 }]);
                }
            }
        }
    }
}

/// Top left of a part's frame in the atlas.
fn cell(row: usize, frame: u16) -> (u32, u32) {
    let k = row as u32 * FRAME_COLS + frame as u32;
    ((k % FRAME_COLS) * FRAME_W, SPRITE_BAND_Y + (k / FRAME_COLS) * FRAME_H)
}

/// What an instance's `mat` holds for a part's frame: its index in the band.
pub fn sprite_index(part: Part, frame: u16) -> u16 {
    let row = PARTS.iter().position(|&p| p == part).expect("every part has a row");
    (row as u32 * FRAME_COLS) as u16 + frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted() -> Vec<u8> {
        let mut px = vec![0u8; (WIDTH * crate::atlas::HEIGHT * 4) as usize];
        paint(&mut px);
        px
    }

    fn opaque(px: &[u8], part: Part, frame: u16) -> Vec<(u32, u32)> {
        let row = PARTS.iter().position(|&p| p == part).unwrap();
        let (ox, oy) = cell(row, frame);
        let mut out = Vec::new();
        for y in 0..FRAME_H {
            for x in 0..FRAME_W {
                if px[(((oy + y) * WIDTH + ox + x) * 4 + 3) as usize] > 0 {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn every_part_is_painted_in_every_frame_in_greys() {
        let px = painted();
        for frame in 0..frames::COUNT {
            for part in PARTS {
                let pixels = opaque(&px, part, frame);
                assert!(!pixels.is_empty(), "{part:?} is missing from frame {frame}");
                let row = PARTS.iter().position(|&p| p == part).unwrap();
                let (ox, oy) = cell(row, frame);
                for (x, y) in pixels {
                    let i = (((oy + y) * WIDTH + ox + x) * 4) as usize;
                    assert!(
                        px[i] == px[i + 1] && px[i + 1] == px[i + 2],
                        "{part:?} must be grey so a tint colours it"
                    );
                }
            }
        }
    }

    #[test]
    fn feet_on_the_ground_touch_the_frame_bottom_and_the_head_stays_inside() {
        let px = painted();
        let grounded = [frames::IDLE].into_iter().chain(frames::WALK).chain(frames::USE);
        for frame in grounded {
            let lowest = opaque(&px, Part::Shoes, frame).iter().map(|p| p.1).max().unwrap();
            assert_eq!(lowest, FRAME_H - 1, "frame {frame} floats or sinks");
        }
        for frame in 0..frames::COUNT {
            let top = opaque(&px, Part::Hair, frame).iter().map(|p| p.1).min().unwrap();
            // The hair shouldn't poke out of the top of the hitbox.
            let figure = (crate::player::HALF[1] * 2.0 * crate::atlas::CELL as f32).round() as u32;
            assert!(top >= FRAME_H - figure, "frame {frame}'s head rises to row {top}");
        }
    }

    #[test]
    fn the_figure_is_stocky_not_skinny() {
        let px = painted();
        let extent = |part, axis: usize| {
            let p = opaque(&px, part, frames::IDLE);
            let v: Vec<u32> = p.iter().map(|q| if axis == 0 { q.0 } else { q.1 }).collect();
            v.iter().max().unwrap() - v.iter().min().unwrap() + 1
        };
        let head_tall = extent(Part::Skin, 1);
        assert!(
            extent(Part::Shirt, 0) >= head_tall,
            "the torso is as wide as the head is tall"
        );
        assert!(head_tall * 3 >= 42, "the head is about a third of the figure");
        assert!(extent(Part::FrontSleeve, 0) >= 5, "arms are thick");
        // Mid-stride, where the legs part, each shin is a solid run at least 7 wide.
        let row = ANKLE_Y as u32 - 3;
        let apart = *frames::WALK
            .iter()
            .max_by(|&&a, &&b| {
                let gap = |f| {
                    let s = skeleton(&pose(f));
                    (s.legs[1][2][0] - s.legs[0][2][0]).abs()
                };
                gap(a).total_cmp(&gap(b))
            })
            .unwrap();
        let mut xs: Vec<u32> = opaque(&px, Part::Trousers, apart)
            .iter()
            .filter(|p| p.1 == row)
            .map(|p| p.0)
            .collect();
        xs.sort();
        let runs = xs.windows(2).filter(|w| w[1] != w[0] + 1).count() + 1;
        assert_eq!(runs, 2, "the legs should part mid-stride: {xs:?}");
        let widest_gap = xs.windows(2).position(|w| w[1] != w[0] + 1).unwrap() + 1;
        assert!(widest_gap >= 7 && xs.len() - widest_gap >= 7, "legs are thick: {xs:?}");
    }

    #[test]
    fn the_swing_arc_eases_out_and_its_frames_follow_it_without_jumps() {
        assert_eq!(swing_arc(0.0), 0.0);
        assert_eq!(swing_arc(1.0), 1.0);
        assert!(swing_arc(0.5) > 0.7, "most of the arc goes by early");
        // The hand moves a little between neighbouring frames, never a leap.
        let hands: Vec<[f32; 2]> = (0..frames::USE.len())
            .map(|k| swing_hand(k as f32 / (frames::USE.len() - 1) as f32).0)
            .collect();
        for w in hands.windows(2) {
            assert!(
                (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) < 3.5,
                "hand jumps between frames: {w:?}"
            );
        }
        // It starts behind and above the shoulder and ends low in front.
        let (start, end) = (swing_hand(0.0).0, swing_hand(1.0).0);
        assert!(start[0] < SHOULDER[0] && start[1] < SHOULDER[1]);
        assert!(end[0] > SHOULDER[0] && end[1] > SHOULDER[1]);
        assert_eq!(use_frame(0.0), frames::USE[0]);
        assert_eq!(use_frame(1.0), *frames::USE.last().unwrap());
    }

    #[test]
    fn walking_moves_the_feet_apart_and_back() {
        let s = |f| skeleton(&pose(f));
        let gap = |f| (s(f).legs[1][2][0] - s(f).legs[0][2][0]).abs();
        let gaps: Vec<f32> = frames::WALK.iter().map(|&f| gap(f)).collect();
        let (lo, hi) = gaps
            .iter()
            .fold((f32::MAX, 0.0f32), |(lo, hi), &g| (lo.min(g), hi.max(g)));
        assert!(hi - lo > 4.0, "a stride should open several pixels: {gaps:?}");
    }

    #[test]
    fn each_part_takes_its_colour_from_the_appearance() {
        let look = Appearance {
            hair: [200, 10, 10],
            shoes: [1, 2, 3],
            ..Appearance::default()
        };
        assert_eq!(look.tint(Part::Hair), [200, 10, 10, 255]);
        assert_eq!(look.tint(Part::Shoes), [1, 2, 3, 255]);
        assert_eq!(look.tint(Part::FrontSleeve), look.tint(Part::Shirt));
        assert_eq!(look.tint(Part::BackHand), look.tint(Part::Skin));
        assert_eq!(look.tint(Part::EyeWhites), [255; 4], "eye whites stay white");
    }

    #[test]
    fn a_sprite_index_finds_its_part_row_and_frame() {
        assert_eq!(sprite_index(Part::BackHand, 3), 3);
        let k = sprite_index(Part::Hair, frames::FALL) as u32;
        assert_eq!(
            cell(PARTS.iter().position(|&p| p == Part::Hair).unwrap(), frames::FALL),
            ((k % FRAME_COLS) * FRAME_W, SPRITE_BAND_Y + (k / FRAME_COLS) * FRAME_H)
        );
    }
}
