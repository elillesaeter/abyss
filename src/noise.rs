//! Recent disturbances the world's inhabitants can react to.
//!
//! Mining is the only source today. Kept as a plain owned struct rather than an
//! ECS resource, so who writes it and who reads it stays visible.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noise {
    pub pos: [f32; 2],
    /// Seconds since the world started, when it happened.
    pub at: f32,
}

/// The last few disturbances, newest last.
#[derive(Default)]
pub struct NoiseLog {
    events: Vec<Noise>,
    now: f32,
}

const KEEP: usize = 8;

impl NoiseLog {
    pub fn advance(&mut self, dt: f32) {
        self.now += dt;
    }

    pub fn record(&mut self, pos: [f32; 2]) {
        if self.events.len() == KEEP {
            self.events.remove(0);
        }
        self.events.push(Noise { pos, at: self.now });
    }

    /// The newest disturbance within `radius` of `listener` and no older than
    /// `max_age` seconds.
    pub fn heard(&self, listener: [f32; 2], radius: f32, max_age: f32) -> Option<Noise> {
        self.events
            .iter()
            .rev()
            .find(|n| {
                self.now - n.at <= max_age
                    && (n.pos[0] - listener[0]).hypot(n.pos[1] - listener[1]) <= radius
            })
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_audible_event_wins() {
        let mut log = NoiseLog::default();
        log.record([0.0, 0.0]);
        log.advance(1.0);
        log.record([5.0, 0.0]);
        assert_eq!(log.heard([0.0, 0.0], 50.0, 10.0).map(|n| n.pos), Some([5.0, 0.0]));
    }

    #[test]
    fn distant_and_stale_events_are_not_heard() {
        let mut log = NoiseLog::default();
        log.record([100.0, 0.0]);
        assert_eq!(log.heard([0.0, 0.0], 40.0, 10.0), None, "too far");

        log.record([1.0, 0.0]);
        log.advance(30.0);
        assert_eq!(log.heard([0.0, 0.0], 40.0, 10.0), None, "too old");
    }

    #[test]
    fn the_log_stays_bounded() {
        let mut log = NoiseLog::default();
        for i in 0..100 {
            log.record([i as f32, 0.0]);
        }
        assert_eq!(log.events.len(), KEEP);
        assert_eq!(log.heard([99.0, 0.0], 2.0, 10.0).map(|n| n.pos), Some([99.0, 0.0]));
    }
}
