//! Test patterns, computed from the geometry of the target.
//!
//! Every pattern scales with the matrix (size, universes, lights per universe) and moves in seconds, so it looks the
//! same at any size and frame rate.

use std::fmt;
use std::time::Duration;

use super::{Source, SourceError};
use crate::fit::Color;
use crate::geometry::{CHANNELS_PER_LIGHT, Geometry};

/// One test pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Each universe in its own colour; a white light runs through every universe at once.
    Universes,
    /// Edges, a grid scaled to the size, a colour per corner, the centre, and a sweeping line.
    Grid,
    /// Hue across, brightness down, drifting slowly.
    Gradient,
    /// Smooth waves of colour.
    Plasma,
    /// One light after the other, in index order.
    Chase,
    /// One colour everywhere.
    Color(Color),
}

impl Pattern {
    pub const NAMES: [&str; 6] = [
        "universes",
        "grid",
        "gradient",
        "plasma",
        "chase",
        "color <rrggbb>",
    ];

    /// Reads a pattern list: names, `color` followed by its colour (`grid color ff0000 chase`).
    pub fn parse_list<S: AsRef<str>>(words: &[S]) -> Result<Vec<Pattern>, SourceError> {
        let mut patterns = Vec::new();
        let mut words = words.iter().map(AsRef::as_ref);
        while let Some(word) = words.next() {
            patterns.push(match word {
                "universes" => Pattern::Universes,
                "grid" => Pattern::Grid,
                "gradient" => Pattern::Gradient,
                "plasma" => Pattern::Plasma,
                "chase" => Pattern::Chase,
                "color" => {
                    let color = words.next().ok_or_else(|| {
                        SourceError::Pattern("color needs a colour, such as: color ff8000".into())
                    })?;
                    Pattern::Color(color.parse().map_err(SourceError::Pattern)?)
                }
                _ => {
                    return Err(SourceError::Pattern(format!(
                        "unknown pattern {word:?}; patterns: {}",
                        Pattern::NAMES.join(", ")
                    )));
                }
            });
        }
        if patterns.is_empty() {
            return Err(SourceError::Pattern(format!(
                "no pattern given; patterns: {}",
                Pattern::NAMES.join(", ")
            )));
        }
        Ok(patterns)
    }

    /// Draws the pattern at `t` seconds into `frame` (RGB, the size of `geometry`).
    pub fn draw(&self, geometry: &Geometry, t: f64, frame: &mut [u8]) {
        assert_eq!(frame.len(), geometry.frame_len());
        let size = geometry.size();
        let (w, h) = (size.width as usize, size.height as usize);
        let mut canvas = Canvas { frame, width: w };
        match *self {
            Pattern::Universes => universes(geometry, t, &mut canvas),
            Pattern::Grid => grid(w, h, t, &mut canvas),
            Pattern::Gradient => {
                for y in 0..h {
                    let value = 0.25 + 0.75 * fraction(y, h);
                    for x in 0..w {
                        let hue = (x as f64 / w as f64 + t / 10.0).fract();
                        canvas.put(x, y, hsv(hue, 1.0, value));
                    }
                }
            }
            Pattern::Plasma => plasma(w, h, t, &mut canvas),
            Pattern::Chase => chase(geometry, t, &mut canvas),
            Pattern::Color(color) => {
                for light in canvas.frame.chunks_mut(CHANNELS_PER_LIGHT) {
                    light.copy_from_slice(&color.0);
                }
            }
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pattern::Universes => f.write_str("universes"),
            Pattern::Grid => f.write_str("grid"),
            Pattern::Gradient => f.write_str("gradient"),
            Pattern::Plasma => f.write_str("plasma"),
            Pattern::Chase => f.write_str("chase"),
            Pattern::Color(color) => write!(f, "color {color}"),
        }
    }
}

struct Canvas<'a> {
    frame: &'a mut [u8],
    width: usize,
}

impl Canvas<'_> {
    fn put(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        self.put_light(y * self.width + x, rgb);
    }

    fn put_light(&mut self, light: usize, rgb: [u8; 3]) {
        let i = light * CHANNELS_PER_LIGHT;
        self.frame[i..i + CHANNELS_PER_LIGHT].copy_from_slice(&rgb);
    }

    fn fill(&mut self, rgb: [u8; 3]) {
        for light in self.frame.chunks_mut(CHANNELS_PER_LIGHT) {
            light.copy_from_slice(&rgb);
        }
    }
}

/// `i` of `n` as 0..=1 (0 when there is a single step).
fn fraction(i: usize, n: usize) -> f64 {
    if n > 1 {
        i as f64 / (n - 1) as f64
    } else {
        0.0
    }
}

/// Hue, saturation and value in 0..=1 to RGB.
fn hsv(hue: f64, saturation: f64, value: f64) -> [u8; 3] {
    let channel = |n: f64| {
        let k = (n + hue * 6.0).rem_euclid(6.0);
        let level = value * (1.0 - saturation * k.min(4.0 - k).clamp(0.0, 1.0));
        (255.0 * level).round() as u8
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

/// Seconds a moving light takes to cross one universe.
const UNIVERSE_PASS: f64 = 3.0;

fn universes(geometry: &Geometry, t: f64, canvas: &mut Canvas) {
    let per_universe = geometry.lights_per_universe() as usize;
    let count = geometry.universe_count();
    let lights = geometry.size().lights();
    let running = ((t / UNIVERSE_PASS).fract() * per_universe as f64) as usize;
    for light in 0..lights {
        // universes carry LEDs in wiring order: colour each light by the universe of its LED
        let led = geometry.led_of_light(light);
        let universe = led / per_universe;
        let rgb = if led % per_universe == running {
            [255, 255, 255]
        } else {
            // neighbours alternate brightness, so that two universes of close hues still part
            let value = if universe % 2 == 0 { 0.9 } else { 0.5 };
            hsv(universe as f64 / count as f64, 1.0, value)
        };
        canvas.put_light(light, rgb);
    }
}

/// Seconds the sweeping line of `grid` takes to cross the matrix.
const SWEEP: f64 = 4.0;

/// Distance from the centre line of a side `n` long: on an even side the centre falls between two lights, which
/// both count as the centre (distance 0), so that lines drawn at equal distances stay symmetric.
fn from_centre(i: usize, n: usize) -> usize {
    let half = n / 2;
    if n % 2 == 1 {
        i.abs_diff(half)
    } else if i >= half {
        i - half
    } else {
        half - 1 - i
    }
}

fn grid(w: usize, h: usize, t: f64, canvas: &mut Canvas) {
    let short = w.min(h);
    // about eight cells along the short side, never finer than every 4 lights; lines start from the centre
    let step = (short / 8).max(4);
    let edge = [255, 80, 0];
    let line = [30, 110, 30];
    for y in 0..h {
        for x in 0..w {
            let on_edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let on_line = from_centre(x, w) % step == 0 || from_centre(y, h) % step == 0;
            canvas.put(
                x,
                y,
                if on_edge {
                    edge
                } else if on_line {
                    line
                } else {
                    [0, 0, 0]
                },
            );
        }
    }
    // the centre: one light, or the two or four around it
    for y in (h - 1) / 2..=h / 2 {
        for x in (w - 1) / 2..=w / 2 {
            canvas.put(x, y, [255, 255, 0]);
        }
    }
    // corners: top-left red, top-right green, bottom-left blue, bottom-right white
    let mark = (short / 10).max(1);
    let corners = [
        (0, 0, [255, 0, 0]),
        (w - mark, 0, [0, 255, 0]),
        (0, h - mark, [0, 0, 255]),
        (w - mark, h - mark, [255, 255, 255]),
    ];
    for (x0, y0, rgb) in corners {
        for y in y0..(y0 + mark).min(h) {
            for x in x0..(x0 + mark).min(w) {
                canvas.put(x, y, rgb);
            }
        }
    }
    let sweep = ((t / SWEEP).fract() * w as f64) as usize;
    for y in 0..h {
        canvas.put(sweep.min(w - 1), y, [255, 255, 255]);
    }
}

fn plasma(w: usize, h: usize, t: f64, canvas: &mut Canvas) {
    // coordinates in units of a fifth of the short side: the same waves at any size
    let unit = w.min(h) as f64 / 5.0;
    let (cx, cy) = (w as f64 / 2.0 / unit, h as f64 / 2.0 / unit);
    for y in 0..h {
        let fy = y as f64 / unit;
        for x in 0..w {
            let fx = x as f64 / unit;
            let v = (fx + t).sin()
                + (1.8 * fy - 1.3 * t).sin()
                + (0.7 * (fx + fy) + 0.7 * t).sin()
                + (1.4 * (fx - cx).hypot(fy - cy) - t).sin();
            canvas.put(x, y, hsv(((v + 4.0) / 8.0 + t / 10.0).fract(), 0.9, 0.85));
        }
    }
}

/// Fewest seconds `chase` takes to run through every light.
const CHASE_MIN_PASS: f64 = 2.0;
/// Most seconds `chase` takes to run through every light.
const CHASE_MAX_PASS: f64 = 20.0;

fn chase(geometry: &Geometry, t: f64, canvas: &mut Canvas) {
    let lights = geometry.size().lights();
    // one universe a second, within a pass of 2 to 20 seconds
    let speed = (geometry.lights_per_universe() as f64).clamp(
        lights as f64 / CHASE_MAX_PASS,
        lights as f64 / CHASE_MIN_PASS,
    );
    let tail = ((speed / 10.0) as usize).clamp(1, lights);
    let head = (t * speed) as usize % lights;
    canvas.fill([0, 0, 0]);
    for k in (0..tail).rev() {
        let light = (head + lights - k) % lights;
        let fade = 1.0 - k as f64 / tail as f64;
        canvas.put_light(light, hsv(light as f64 / lights as f64, 1.0, fade));
    }
}

/// Patterns in turn, each shown `each` (forever when there is a single one).
pub struct PatternSource {
    patterns: Vec<Pattern>,
    each: Duration,
    geometry: Geometry,
    frame: Vec<u8>,
}

impl PatternSource {
    pub fn new(
        patterns: Vec<Pattern>,
        each: Duration,
        geometry: Geometry,
    ) -> Result<Self, SourceError> {
        if patterns.is_empty() {
            return Err(SourceError::Pattern("no pattern given".into()));
        }
        if patterns.len() > 1 && each.is_zero() {
            return Err(SourceError::Pattern(
                "several patterns need a time for each".into(),
            ));
        }
        Ok(PatternSource {
            patterns,
            each,
            geometry,
            frame: vec![0; geometry.frame_len()],
        })
    }

    /// The pattern shown at `t`, and `t` from the start of its turn.
    pub fn at(&self, t: Duration) -> (Pattern, Duration) {
        if self.patterns.len() == 1 {
            return (self.patterns[0], t);
        }
        let each = self.each.as_nanos();
        let turn = (t.as_nanos() / each) as usize % self.patterns.len();
        let into = Duration::from_nanos((t.as_nanos() % each) as u64);
        (self.patterns[turn], into)
    }
}

impl Source for PatternSource {
    fn frame(&mut self, t: Duration) -> Result<Option<&[u8]>, SourceError> {
        let (pattern, into) = self.at(t);
        pattern.draw(&self.geometry, into.as_secs_f64(), &mut self.frame);
        Ok(Some(&self.frame))
    }

    fn describe(&self) -> String {
        let names: Vec<String> = self.patterns.iter().map(Pattern::to_string).collect();
        if self.patterns.len() == 1 {
            format!("pattern {}", names[0])
        } else {
            format!(
                "pattern {} ({} s each)",
                names.join(", "),
                self.each.as_secs_f64()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;

    fn geometry(width: u32, height: u32, per_universe: u16) -> Geometry {
        Geometry::new(Size { width, height }, 1, per_universe).unwrap()
    }

    fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * width + x) * 3) as usize;
        frame[i..i + 3].try_into().unwrap()
    }

    const ALL: [Pattern; 6] = [
        Pattern::Universes,
        Pattern::Grid,
        Pattern::Gradient,
        Pattern::Plasma,
        Pattern::Chase,
        Pattern::Color(Color([1, 2, 3])),
    ];

    #[test]
    fn every_pattern_draws_at_any_size() {
        for (w, h, per_universe) in [
            (1, 1, 170),
            (7, 3, 170),
            (8, 8, 1),
            (3, 200, 64),
            (192, 64, 170),
            (300, 1, 100),
        ] {
            let geometry = geometry(w, h, per_universe);
            let mut frame = vec![0; geometry.frame_len()];
            for pattern in ALL {
                for t in [0.0, 0.5, 3.99, 1234.5] {
                    pattern.draw(&geometry, t, &mut frame);
                }
            }
        }
    }

    #[test]
    fn universes_change_colour_at_each_boundary() {
        let geometry = geometry(10, 10, 30); // 100 lights: universes of 30, 30, 30 and 10
        let mut frame = vec![0; geometry.frame_len()];
        Pattern::Universes.draw(&geometry, 1.5, &mut frame); // the white light is at light 15 of each
        let light = |i: usize| -> [u8; 3] { frame[i * 3..i * 3 + 3].try_into().unwrap() };
        for universe in 0..4 {
            let first = universe * 30;
            assert_ne!(light(first), [255, 255, 255]);
            if universe < 3 {
                assert_eq!(
                    light(first),
                    light(first + 29),
                    "one colour across universe {universe}"
                );
                assert_eq!(light(first + 15), [255, 255, 255]);
            }
            if universe > 0 {
                assert_ne!(
                    light(first),
                    light(first - 1),
                    "a new colour at universe {universe}"
                );
            }
        }
    }

    #[test]
    fn grid_marks_the_corners_and_the_centre() {
        let geometry = geometry(40, 20, 170);
        let mut frame = vec![0; geometry.frame_len()];
        Pattern::Grid.draw(&geometry, 2.0, &mut frame); // sweep at x = 20
        assert_eq!(pixel(&frame, 40, 0, 0), [255, 0, 0]);
        assert_eq!(pixel(&frame, 40, 39, 0), [0, 255, 0]);
        assert_eq!(pixel(&frame, 40, 0, 19), [0, 0, 255]);
        assert_eq!(pixel(&frame, 40, 39, 19), [255, 255, 255]);
        assert_eq!(pixel(&frame, 40, 19, 9), [255, 255, 0]);
        assert_eq!(pixel(&frame, 40, 20, 5), [255, 255, 255]);
    }

    #[test]
    fn grid_is_symmetric_about_the_centre() {
        for (w, h) in [(8u32, 8u32), (16, 16), (32, 8), (7, 5), (192, 64), (9, 4)] {
            let geometry = geometry(w, h, 170);
            let mut frame = vec![0; geometry.frame_len()];
            Pattern::Grid.draw(&geometry, 0.0, &mut frame); // the sweep at x = 0, on the edge
            let corner = |x: u32, y: u32| {
                let mark = (w.min(h) / 10).max(1);
                (x < mark || x >= w - mark) && (y < mark || y >= h - mark)
            };
            for y in 0..h {
                for x in 1..w - 1 {
                    if corner(x, y) || corner(w - 1 - x, y) || corner(x, h - 1 - y) {
                        continue;
                    }
                    let here = pixel(&frame, w, x, y);
                    assert_eq!(
                        here,
                        pixel(&frame, w, w - 1 - x, y),
                        "{w}x{h}: ({x}, {y}) left-right"
                    );
                    assert_eq!(
                        here,
                        pixel(&frame, w, x, h - 1 - y),
                        "{w}x{h}: ({x}, {y}) top-bottom"
                    );
                }
            }
        }
        // 8x8: the centre cross is two lights wide, on lights 3 and 4
        let geometry = geometry(8, 8, 170);
        let mut frame = vec![0; geometry.frame_len()];
        Pattern::Grid.draw(&geometry, 0.0, &mut frame);
        let line = [30, 110, 30];
        assert_eq!(pixel(&frame, 8, 3, 1), line);
        assert_eq!(pixel(&frame, 8, 4, 1), line);
        assert_eq!(pixel(&frame, 8, 2, 1), [0, 0, 0]);
        assert_eq!(pixel(&frame, 8, 1, 3), line);
    }

    #[test]
    fn chase_runs_in_index_order() {
        let geometry = geometry(16, 16, 128); // 256 lights at 128 a second
        let mut frame = vec![0; geometry.frame_len()];
        let brightest = |frame: &[u8]| {
            (0..256)
                .max_by_key(|i| {
                    frame[i * 3..i * 3 + 3]
                        .iter()
                        .map(|&c| u32::from(c))
                        .sum::<u32>()
                })
                .unwrap()
        };
        Pattern::Chase.draw(&geometry, 0.5, &mut frame);
        assert_eq!(brightest(&frame), 64);
        Pattern::Chase.draw(&geometry, 1.0, &mut frame);
        assert_eq!(brightest(&frame), 128);
        // lights behind the tail are off
        assert_eq!(&frame[..60 * 3], &[0; 180][..]);
    }

    #[test]
    fn colour_fills_every_light() {
        let geometry = geometry(3, 2, 170);
        let mut frame = vec![0; geometry.frame_len()];
        Pattern::Color(Color([9, 8, 7])).draw(&geometry, 0.0, &mut frame);
        assert_eq!(frame, [9, 8, 7].repeat(6));
    }

    #[test]
    fn lists_read_with_colours() {
        assert_eq!(
            Pattern::parse_list(&["grid", "color", "ff0000", "chase"]).unwrap(),
            [
                Pattern::Grid,
                Pattern::Color(Color([255, 0, 0])),
                Pattern::Chase
            ]
        );
        assert!(Pattern::parse_list(&["color"]).is_err());
        assert!(Pattern::parse_list(&["color", "red"]).is_err());
        assert!(Pattern::parse_list(&["solid"]).is_err());
        assert!(Pattern::parse_list::<&str>(&[]).is_err());
    }

    #[test]
    fn several_patterns_take_turns() {
        let geometry = geometry(4, 4, 170);
        let source = PatternSource::new(
            vec![Pattern::Grid, Pattern::Plasma, Pattern::Chase],
            Duration::from_secs(10),
            geometry,
        )
        .unwrap();
        assert_eq!(
            source.at(Duration::from_secs(0)),
            (Pattern::Grid, Duration::ZERO)
        );
        assert_eq!(
            source.at(Duration::from_secs(12)),
            (Pattern::Plasma, Duration::from_secs(2))
        );
        assert_eq!(
            source.at(Duration::from_secs(35)),
            (Pattern::Grid, Duration::from_secs(5))
        );
        let single = PatternSource::new(vec![Pattern::Plasma], Duration::ZERO, geometry).unwrap();
        assert_eq!(
            single.at(Duration::from_secs(99)),
            (Pattern::Plasma, Duration::from_secs(99))
        );
        assert!(
            PatternSource::new(
                vec![Pattern::Grid, Pattern::Chase],
                Duration::ZERO,
                geometry
            )
            .is_err()
        );
    }
}
