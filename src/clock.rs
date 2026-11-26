//! Fixed-deadline pacing: tick `n` is due at `start + n / fps`, so the frame rate does not drift, and a late tick
//! does not push the following ones.

use std::time::{Duration, Instant};

/// A tick of the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// Ticks since the start, skipped ones counted.
    pub index: u64,
    /// When the tick was due, from the start.
    pub at: Duration,
    /// Ticks given up because the previous one ended too late.
    pub skipped: u64,
}

#[derive(Clone, Debug)]
pub struct Clock {
    start: Instant,
    fps: f64,
    next: u64,
}

impl Clock {
    pub fn new(fps: f64, start: Instant) -> Self {
        assert!(fps > 0.0, "a frame rate above 0");
        Clock {
            start,
            fps,
            next: 0,
        }
    }

    pub fn start(&self) -> Instant {
        self.start
    }

    /// When tick `index` is due, from the start.
    pub fn due(&self, index: u64) -> Duration {
        Duration::from_secs_f64(index as f64 / self.fps)
    }

    /// The next tick as of `now`: the next one in turn, or, when more than a whole period late, the latest one due
    /// (the ones in between are skipped rather than sent in a burst).
    pub fn next_tick(&mut self, now: Instant) -> Tick {
        let elapsed = now.saturating_duration_since(self.start);
        let mut index = self.next;
        let mut skipped = 0;
        if elapsed > self.due(index + 1) {
            let latest = (elapsed.as_secs_f64() * self.fps).floor() as u64;
            skipped = latest - index;
            index = latest;
        }
        self.next = index + 1;
        Tick {
            index,
            at: self.due(index),
            skipped,
        }
    }

    /// Sleeps until the next tick is due, and returns it.
    pub fn wait(&mut self) -> Tick {
        let tick = self.next_tick(Instant::now());
        let deadline = self.start + tick.at;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_come_at_fixed_deadlines() {
        let start = Instant::now();
        let mut clock = Clock::new(30.0, start);
        for i in 0..90 {
            // a little late every time: the deadlines do not move
            let now = start + clock.due(i) + Duration::from_millis(5);
            let tick = clock.next_tick(now);
            assert_eq!((tick.index, tick.skipped), (i, 0));
        }
        assert_eq!(clock.due(90), Duration::from_secs(3));
    }

    #[test]
    fn a_long_stall_skips_ticks_instead_of_bursting() {
        let start = Instant::now();
        let mut clock = Clock::new(10.0, start);
        assert_eq!(clock.next_tick(start).index, 0);
        // stalled until 1.05 s: ticks 1 to 9 are skipped, tick 10 is the one due
        let tick = clock.next_tick(start + Duration::from_millis(1050));
        assert_eq!((tick.index, tick.skipped), (10, 9));
        assert_eq!(tick.at, Duration::from_secs(1));
        assert_eq!(
            clock.next_tick(start + Duration::from_millis(1060)).index,
            11
        );
    }

    #[test]
    fn wait_sleeps_until_the_deadline() {
        let mut clock = Clock::new(50.0, Instant::now());
        let first = clock.wait();
        let second = clock.wait();
        assert_eq!((first.index, second.index), (0, 1));
        assert!(clock.start().elapsed() >= Duration::from_millis(20));
    }
}
