//! Time of day: where the sun and moon are, and what light they and the sky give.
//!
//! Light outdoors is two things. Direct light comes from one body in one
//! direction: sharp, blockable, casting shadows. Ambient light comes from the whole
//! sky: soft, filling those shadows in. Each has its own colour through the day,
//! and the contrast between them is the look — a warm low sun against blue-purple
//! shade is a sunset, a cold moon over near-black is a full-moon night.

/// One full day, in seconds: 15 minutes of day and 9 of night, as Terraria has it.
pub const DAY_SECONDS: f32 = 24.0 * 60.0;
/// Share of the cycle the sun is up.
const DAYTIME: f32 = 15.0 / 24.0;
/// The clock reads 4:30 at sunrise, so 15 hours of daylight end at 19:30.
const SUNRISE_HOUR: f32 = 4.5;
/// The sun and moon swing this far either side of straight up, in radians: low at
/// either end of their arc, so dawn and dusk light comes in at a steep slant.
pub const ARC: f32 = 1.45;
const MOON_PHASES: u32 = 8;
/// The first night's moon is full.
const FIRST_PHASE: u32 = 4;
/// Direct moonlight at full moon: cool, and bright enough to see by and to cast
/// clear shadows. The moon's phase scales it down to nothing at new moon.
const MOONLIGHT: [f32; 3] = [0.30, 0.37, 0.50];
/// Ambient added by a moonlit sky, scaled the same way.
const MOON_SKY: [f32; 3] = [0.025, 0.03, 0.045];
/// How long moonlight takes to fade in after dusk and out before dawn, as a share
/// of the night.
const MOON_FADE: f32 = 0.1;
/// Morning mist gathers over the last stretch of the night and burns off this
/// long after sunrise, both as shares of the cycle: about 8:00 it is gone.
const MIST_GATHERS: f32 = 0.06;
const MIST_LIFTS: f32 = 0.15;

#[derive(Clone, Copy, Debug, Default)]
pub struct Sky {
    /// Seconds since the first sunrise.
    pub time: f32,
}

/// What the sky gives at one moment.
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    /// Unit vector from the ground toward the light, y down, so up is negative.
    pub toward: [f32; 2],
    pub direct: [f32; 3],
    pub ambient: [f32; 3],
    /// Sky gradient, in sRGB.
    pub sky_top: [f32; 3],
    pub sky_horizon: [f32; 3],
    /// How visible the stars are, 0..1.
    pub stars: f32,
    /// Angle of each body from straight up, while it is above the horizon.
    pub sun: Option<f32>,
    pub moon: Option<f32>,
    /// Where the moon is in its cycle, 0..1: 0 new, 0.5 full.
    pub phase: f32,
    /// How thick the morning mist lies in the valleys, 0..1.
    pub mist: f32,
}

/// One point of the day's palette. Between keys, everything blends linearly.
struct Key {
    /// Share of the cycle since sunrise.
    at: f32,
    direct: [f32; 3],
    ambient: [f32; 3],
    top: [f32; 3],
    horizon: [f32; 3],
    stars: f32,
}

const DAY: Key = Key {
    at: 0.0,
    direct: [0.62, 0.59, 0.53],
    ambient: [0.38, 0.41, 0.47],
    top: [0.24, 0.44, 0.77],
    horizon: [0.70, 0.84, 0.93],
    stars: 0.0,
};
const NIGHT: Key = Key {
    at: 0.0,
    direct: [0.0; 3],
    ambient: [0.02, 0.022, 0.035],
    top: [0.01, 0.015, 0.05],
    horizon: [0.04, 0.06, 0.13],
    stars: 1.0,
};

const KEYS: [Key; 9] = [
    // Sunrise: a cooler pink-gold than sunset.
    Key {
        at: 0.0,
        direct: [0.80, 0.50, 0.38],
        ambient: [0.26, 0.22, 0.34],
        top: [0.30, 0.33, 0.62],
        horizon: [0.98, 0.66, 0.58],
        stars: 0.15,
    },
    Key { at: 0.06, ..DAY },
    Key { at: 0.56, ..DAY },
    // Sunset: an orange sun under a pink and purple sky, shade gone violet.
    Key {
        at: 0.615,
        direct: [0.95, 0.45, 0.22],
        ambient: [0.34, 0.20, 0.32],
        top: [0.42, 0.26, 0.52],
        horizon: [1.0, 0.56, 0.40],
        stars: 0.0,
    },
    Key {
        at: 0.64,
        direct: [0.40, 0.14, 0.10],
        ambient: [0.16, 0.09, 0.18],
        top: [0.18, 0.12, 0.32],
        horizon: [0.62, 0.30, 0.38],
        stars: 0.3,
    },
    Key { at: 0.67, ..NIGHT },
    Key { at: 0.96, ..NIGHT },
    // Before dawn the horizon greys and warms before the sun is up.
    Key {
        at: 0.985,
        direct: [0.10, 0.05, 0.05],
        ambient: [0.10, 0.09, 0.14],
        top: [0.10, 0.12, 0.32],
        horizon: [0.48, 0.34, 0.42],
        stars: 0.5,
    },
    Key {
        at: 1.0,
        direct: [0.80, 0.50, 0.38],
        ambient: [0.26, 0.22, 0.34],
        top: [0.30, 0.33, 0.62],
        horizon: [0.98, 0.66, 0.58],
        stars: 0.15,
    },
];

impl Sky {
    /// A sky at `hour` (0..24, Terraria's clock) on day `day`. Day 0's night has
    /// the first phase, and each night after it the next.
    pub fn at(day: u32, hour: f32) -> Sky {
        let since_sunrise = (hour - SUNRISE_HOUR).rem_euclid(24.0) / 24.0;
        Sky {
            time: (day as f32 + since_sunrise) * DAY_SECONDS,
        }
    }

    pub fn advance(&mut self, dt: f32) {
        self.time += dt;
    }

    pub fn lighting(&self) -> Lighting {
        let t = (self.time / DAY_SECONDS).rem_euclid(1.0);
        let day = (self.time / DAY_SECONDS).floor().max(0.0) as u32;
        let key = blend(t);

        // Day runs along the sun's arc; night along the moon's.
        let sun_angle = -ARC + 2.0 * ARC * (t / DAYTIME).min(1.0);
        let night = ((t - DAYTIME) / (1.0 - DAYTIME)).max(0.0);
        let moon_angle = -ARC + 2.0 * ARC * night;
        let phase = ((FIRST_PHASE + day) % MOON_PHASES) as f32 / MOON_PHASES as f32;
        let lit = (1.0 - (phase * std::f32::consts::TAU).cos()) / 2.0;
        let fade = if t < DAYTIME {
            0.0
        } else {
            smoothstep(night / MOON_FADE) * smoothstep((1.0 - night) / MOON_FADE)
        };
        let moonlight = MOONLIGHT.map(|c| c * lit * fade);

        let direct = add(key.direct, moonlight);
        // Whichever body gives more light sets the direction; at dusk both are dim.
        let angle = if luminance(moonlight) > luminance(key.direct) {
            moon_angle
        } else {
            sun_angle
        };
        Lighting {
            toward: [angle.sin(), -angle.cos()],
            direct,
            ambient: add(key.ambient, MOON_SKY.map(|c| c * lit * fade)),
            sky_top: key.top,
            sky_horizon: key.horizon,
            // A bright moon washes the faintest stars out.
            stars: key.stars * (1.0 - 0.4 * lit * fade),
            sun: (t < DAYTIME).then_some(sun_angle),
            moon: (t >= DAYTIME).then_some(moon_angle),
            phase,
            mist: smoothstep((t - (1.0 - MIST_GATHERS)) / MIST_GATHERS).max(1.0 - smoothstep(t / MIST_LIFTS)),
        }
    }
}

fn blend(t: f32) -> Key {
    let i = KEYS.iter().rposition(|k| k.at <= t).unwrap_or(0).min(KEYS.len() - 2);
    let (a, b) = (&KEYS[i], &KEYS[i + 1]);
    let f = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0);
    let mix = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|c| x[c] + (y[c] - x[c]) * f);
    Key {
        at: t,
        direct: mix(a.direct, b.direct),
        ambient: mix(a.ambient, b.ambient),
        top: mix(a.top, b.top),
        horizon: mix(a.horizon, b.horizon),
        stars: a.stars + (b.stars - a.stars) * f,
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mist_lies_thick_at_dawn_and_is_gone_by_mid_morning() {
        let mist = |hour: f32| Sky::at(1, hour).lighting().mist;
        assert!(mist(SUNRISE_HOUR) > 0.99, "thickest at sunrise");
        assert!(mist(3.0) < mist(4.2) && mist(4.2) > 0.5, "it gathers before dawn");
        assert!(mist(6.0) > 0.0 && mist(6.0) < mist(5.0), "it thins as the sun climbs");
        for hour in [9.0, 12.0, 17.0, 22.0, 1.0] {
            assert_eq!(mist(hour), 0.0, "no mist at {hour}:00");
        }
    }

    fn total(l: &Lighting) -> f32 {
        luminance(l.direct) + luminance(l.ambient)
    }

    #[test]
    fn the_sun_rises_in_the_east_peaks_at_noon_and_sets_in_the_west() {
        let dawn = Sky::at(0, 5.0).lighting();
        let noon = Sky::at(0, 12.0).lighting();
        let dusk = Sky::at(0, 19.0).lighting();
        assert!(
            dawn.toward[0] < -0.9,
            "the morning sun is low on the left, {:?}",
            dawn.toward
        );
        assert!(
            noon.toward[1] < -0.99,
            "noon light comes from nearly straight up, {:?}",
            noon.toward
        );
        assert!(
            dusk.toward[0] > 0.9,
            "the evening sun is low on the right, {:?}",
            dusk.toward
        );
        assert!(noon.sun.is_some() && noon.moon.is_none());
        assert!(
            Sky::at(0, 0.0).lighting().moon.is_some(),
            "midnight belongs to the moon"
        );
    }

    #[test]
    fn a_day_lasts_24_minutes_with_15_of_daylight() {
        let mut sky = Sky::default();
        let mut daylit = 0;
        for _ in 0..24 * 60 {
            daylit += sky.lighting().sun.is_some() as u32;
            sky.advance(1.0);
        }
        assert_eq!(daylit, 15 * 60);
        assert_eq!(sky.time, DAY_SECONDS);
    }

    #[test]
    fn noon_is_full_daylight_and_sunset_is_warm_against_violet_shade() {
        let noon = Sky::at(0, 12.0).lighting();
        assert!(
            (total(&noon) - 1.0).abs() < 0.05,
            "noon should be about full light, got {}",
            total(&noon)
        );
        let sunset = Sky::at(0, 19.2).lighting();
        assert!(
            sunset.direct[0] > 1.6 * sunset.direct[2],
            "the low sun is orange: {:?}",
            sunset.direct
        );
        assert!(
            sunset.ambient[2] >= sunset.ambient[1],
            "the shade is violet, not orange: {:?}",
            sunset.ambient
        );
    }

    #[test]
    fn the_moon_sets_how_dark_the_night_is() {
        // Day 0 is full; four nights later the moon is new.
        let full = Sky::at(0, 0.0).lighting();
        let new = Sky::at(4, 0.0).lighting();
        assert_eq!(full.phase, 0.5);
        assert_eq!(new.phase, 0.0);
        assert!(
            total(&new) < 0.04,
            "a new moon night is nearly black, got {}",
            total(&new)
        );
        assert!(
            total(&full) > 0.3,
            "a full moon is enough to see by, got {}",
            total(&full)
        );
        assert!(
            luminance(full.direct) > 4.0 * luminance(full.ambient),
            "full moonlight is mostly direct, so its shadows are clear"
        );
        let waxing = Sky::at(2, 0.0).lighting();
        assert!(total(&new) < total(&waxing) && total(&waxing) < total(&full));
    }

    #[test]
    fn the_palette_has_no_jumps() {
        let mut sky = Sky::default();
        let mut last = sky.lighting();
        for _ in 0..2 * 24 * 60 {
            sky.advance(1.0);
            let now = sky.lighting();
            let step = (total(&now) - total(&last)).abs();
            assert!(step < 0.05, "light jumped by {step} at {}s", sky.time);
            last = now;
        }
    }
}
