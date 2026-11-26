//! Size of a matrix, its wiring and its split into universes.
//!
//! A picture is RGB, three bytes per light, lights in rows from the top-left corner (index = y × width + x). The
//! wiring says in which order the LEDs follow each other (rows or columns, straight or serpentine, from which
//! corner): before sending, a picture is put in that order. Each universe then carries `lights_per_universe` LEDs;
//! the last one carries what is left.

use std::fmt;
use std::str::FromStr;

use crate::sacn;

/// Channels per light: R, G, B.
pub const CHANNELS_PER_LIGHT: usize = 3;
/// The most lights a universe can carry (512 / 3).
pub const MAX_LIGHTS_PER_UNIVERSE: u16 = (sacn::MAX_CHANNELS / CHANNELS_PER_LIGHT) as u16;

/// Width and height in lights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn lights(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

impl FromStr for Size {
    type Err = GeometryError;

    /// Reads `<width>x<height>`, such as `192x64`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || GeometryError::Size(text.to_owned());
        let (width, height) = text.trim().split_once(['x', 'X']).ok_or_else(bad)?;
        let width: u32 = width.trim().parse().map_err(|_| bad())?;
        let height: u32 = height.trim().parse().map_err(|_| bad())?;
        if width == 0 || height == 0 {
            return Err(bad());
        }
        Ok(Size { width, height })
    }
}

/// A matrix and the universes that carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    size: Size,
    first_universe: u16,
    lights_per_universe: u16,
    wiring: Wiring,
}

impl Geometry {
    pub fn new(
        size: Size,
        first_universe: u16,
        lights_per_universe: u16,
    ) -> Result<Self, GeometryError> {
        if size.width == 0 || size.height == 0 {
            return Err(GeometryError::Size(size.to_string()));
        }
        if !(1..=MAX_LIGHTS_PER_UNIVERSE).contains(&lights_per_universe) {
            return Err(GeometryError::LightsPerUniverse(lights_per_universe));
        }
        if !sacn::UNIVERSES.contains(&first_universe) {
            return Err(GeometryError::FirstUniverse(first_universe));
        }
        let geometry = Geometry {
            size,
            first_universe,
            lights_per_universe,
            wiring: Wiring::default(),
        };
        let last = first_universe as usize + geometry.universe_count() - 1;
        if last > *sacn::UNIVERSES.end() as usize {
            return Err(GeometryError::TooManyUniverses {
                first: first_universe,
                count: geometry.universe_count(),
            });
        }
        Ok(geometry)
    }

    /// The same geometry with LEDs wired as `wiring`.
    pub fn with_wiring(self, wiring: Wiring) -> Self {
        Geometry { wiring, ..self }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn wiring(&self) -> Wiring {
        self.wiring
    }

    /// The light (index in a picture, y × width + x) that LED `led` shows.
    pub fn light_of_led(&self, led: usize) -> usize {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let serpentine = self.wiring.layout.is_serpentine();
        // coordinates as seen from the origin corner
        let (cx, cy) = if self.wiring.layout.is_columns() {
            let (column, along) = (led / h, led % h);
            (
                column,
                if serpentine && column % 2 == 1 {
                    h - 1 - along
                } else {
                    along
                },
            )
        } else {
            let (row, along) = (led / w, led % w);
            (
                if serpentine && row % 2 == 1 {
                    w - 1 - along
                } else {
                    along
                },
                row,
            )
        };
        let (x, y) = self.wiring.origin.flip(cx, cy, w, h);
        y * w + x
    }

    /// The LED that shows light `light` (index in a picture, y × width + x).
    pub fn led_of_light(&self, light: usize) -> usize {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let serpentine = self.wiring.layout.is_serpentine();
        let (cx, cy) = self.wiring.origin.flip(light % w, light / w, w, h);
        if self.wiring.layout.is_columns() {
            cx * h
                + if serpentine && cx % 2 == 1 {
                    h - 1 - cy
                } else {
                    cy
                }
        } else {
            cy * w
                + if serpentine && cy % 2 == 1 {
                    w - 1 - cx
                } else {
                    cx
                }
        }
    }

    /// `frame` (a picture) in wiring order, in `out`; `None` when the wiring is the picture's own order (rows from
    /// the top-left corner), so that `frame` goes out as it is.
    pub fn to_wiring_order<'a>(&self, frame: &'a [u8], out: &'a mut Vec<u8>) -> &'a [u8] {
        if self.wiring == Wiring::default() {
            return frame;
        }
        out.clear();
        out.reserve(frame.len());
        for led in 0..self.size.lights() {
            let at = self.light_of_led(led) * CHANNELS_PER_LIGHT;
            out.extend_from_slice(&frame[at..at + CHANNELS_PER_LIGHT]);
        }
        out
    }

    pub fn first_universe(&self) -> u16 {
        self.first_universe
    }

    pub fn lights_per_universe(&self) -> u16 {
        self.lights_per_universe
    }

    /// Bytes of an RGB picture of this size.
    pub fn frame_len(&self) -> usize {
        self.size.lights() * CHANNELS_PER_LIGHT
    }

    pub fn universe_count(&self) -> usize {
        self.size
            .lights()
            .div_ceil(self.lights_per_universe as usize)
    }

    /// The universe numbers, in order.
    pub fn universes(&self) -> impl Iterator<Item = u16> + use<> {
        let first = self.first_universe;
        (0..self.universe_count()).map(move |i| first + i as u16)
    }

    /// Splits LED values in wiring order (see [`to_wiring_order`](Self::to_wiring_order)) into `(universe, DMX
    /// values)`, the values borrowed from `frame`.
    ///
    /// Panics if `frame` is not [`frame_len`](Self::frame_len) bytes long: pictures are fitted to the matrix
    /// before they get here.
    pub fn split<'a>(&self, frame: &'a [u8]) -> impl Iterator<Item = (u16, &'a [u8])> + use<'a> {
        assert_eq!(
            frame.len(),
            self.frame_len(),
            "a {} picture has {} bytes",
            self.size,
            self.frame_len()
        );
        let chunk = self.lights_per_universe as usize * CHANNELS_PER_LIGHT;
        self.universes().zip(frame.chunks(chunk))
    }
}

/// The order LEDs follow each other in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wiring {
    pub layout: Layout,
    /// The corner of the first LED.
    pub origin: Origin,
}

/// How LEDs run across the matrix, from the origin corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Row after row, each in the same direction.
    #[default]
    Rows,
    /// Row after row, every other one running back.
    SerpentineRows,
    /// Column after column, each in the same direction.
    Columns,
    /// Column after column, every other one running back.
    SerpentineColumns,
}

impl Layout {
    pub const NAMES: [&str; 4] = ["rows", "serpentine-rows", "columns", "serpentine-columns"];

    fn is_serpentine(self) -> bool {
        matches!(self, Layout::SerpentineRows | Layout::SerpentineColumns)
    }

    fn is_columns(self) -> bool {
        matches!(self, Layout::Columns | Layout::SerpentineColumns)
    }
}

impl FromStr for Layout {
    type Err = GeometryError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text {
            "rows" => Layout::Rows,
            "serpentine-rows" => Layout::SerpentineRows,
            "columns" => Layout::Columns,
            "serpentine-columns" => Layout::SerpentineColumns,
            _ => return Err(GeometryError::Layout(text.to_owned())),
        })
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = Layout::NAMES;
        f.write_str(match self {
            Layout::Rows => names[0],
            Layout::SerpentineRows => names[1],
            Layout::Columns => names[2],
            Layout::SerpentineColumns => names[3],
        })
    }
}

/// The corner of the first LED, seen from the front of the matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Origin {
    pub const NAMES: [&str; 4] = ["top-left", "top-right", "bottom-left", "bottom-right"];

    /// Coordinates seen from this corner to coordinates from the top-left corner, and back (the same flips).
    fn flip(self, x: usize, y: usize, width: usize, height: usize) -> (usize, usize) {
        let right = matches!(self, Origin::TopRight | Origin::BottomRight);
        let bottom = matches!(self, Origin::BottomLeft | Origin::BottomRight);
        (
            if right { width - 1 - x } else { x },
            if bottom { height - 1 - y } else { y },
        )
    }
}

impl FromStr for Origin {
    type Err = GeometryError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text {
            "top-left" => Origin::TopLeft,
            "top-right" => Origin::TopRight,
            "bottom-left" => Origin::BottomLeft,
            "bottom-right" => Origin::BottomRight,
            _ => return Err(GeometryError::Origin(text.to_owned())),
        })
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = Origin::NAMES;
        f.write_str(match self {
            Origin::TopLeft => names[0],
            Origin::TopRight => names[1],
            Origin::BottomLeft => names[2],
            Origin::BottomRight => names[3],
        })
    }
}

/// A size or universe layout that cannot be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryError {
    Size(String),
    LightsPerUniverse(u16),
    FirstUniverse(u16),
    TooManyUniverses { first: u16, count: usize },
    Layout(String),
    Origin(String),
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::Size(text) => {
                write!(
                    f,
                    "a size is <width>x<height>, both above 0, such as 192x64: {text:?}"
                )
            }
            GeometryError::LightsPerUniverse(n) => write!(
                f,
                "lights per universe must be from 1 to {MAX_LIGHTS_PER_UNIVERSE}: {n}"
            ),
            GeometryError::FirstUniverse(n) => write!(
                f,
                "the first universe must be from {} to {}: {n}",
                sacn::UNIVERSES.start(),
                sacn::UNIVERSES.end()
            ),
            GeometryError::TooManyUniverses { first, count } => write!(
                f,
                "{count} universes from universe {first} go past universe {}",
                sacn::UNIVERSES.end()
            ),
            GeometryError::Layout(text) => {
                write!(f, "unknown layout {text:?}: {}", Layout::NAMES.join(", "))
            }
            GeometryError::Origin(text) => {
                write!(f, "unknown origin {text:?}: {}", Origin::NAMES.join(", "))
            }
        }
    }
}

impl std::error::Error for GeometryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Geometry {
        Geometry::new("192x64".parse().unwrap(), 1, 170).unwrap()
    }

    #[test]
    fn sizes_read_and_print() {
        assert_eq!(
            "192x64".parse::<Size>(),
            Ok(Size {
                width: 192,
                height: 64
            })
        );
        assert_eq!("32X8".parse::<Size>().unwrap().to_string(), "32x8");
        for bad in ["", "192", "x64", "192x", "0x64", "192x0", "-1x4", "a x b"] {
            assert!(bad.parse::<Size>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_192_by_64_matrix_needs_73_universes() {
        let geometry = wall();
        assert_eq!(geometry.universe_count(), 73);
        assert_eq!(geometry.universes().next(), Some(1));
        assert_eq!(geometry.universes().last(), Some(73));
    }

    #[test]
    fn split_keeps_every_byte_in_order() {
        let geometry = wall();
        let frame: Vec<u8> = (0..geometry.frame_len()).map(|i| (i * 7) as u8).collect();
        let parts: Vec<_> = geometry.split(&frame).collect();
        assert_eq!(parts.len(), 73);
        assert!(parts[..72].iter().all(|(_, values)| values.len() == 510));
        // 12288 lights - 72 × 170 = 48 lights in the last universe
        assert_eq!(parts[72], (73, &frame[72 * 510..]));
        assert_eq!(parts[72].1.len(), 48 * 3);
        let joined: Vec<u8> = parts
            .iter()
            .flat_map(|(_, values)| values.iter().copied())
            .collect();
        assert_eq!(joined, frame);
        // light (x, y) = (5, 1) is light 197: universe 2, light 27
        let at = (192 + 5) * 3;
        assert_eq!(parts[1].1[27 * 3], frame[at]);
    }

    #[test]
    fn other_layouts_split_by_their_own_count() {
        let geometry = Geometry::new("16x16".parse().unwrap(), 10, 128).unwrap();
        let frame = vec![1; geometry.frame_len()];
        let parts: Vec<_> = geometry.split(&frame).collect();
        assert_eq!(
            parts.iter().map(|(u, v)| (*u, v.len())).collect::<Vec<_>>(),
            [(10, 384), (11, 384)]
        );
    }

    #[test]
    #[should_panic(expected = "a 192x64 picture has 36864 bytes")]
    fn split_refuses_a_frame_of_another_size() {
        let _ = wall().split(&[0; 3]).count();
    }

    fn wired(width: u32, height: u32, layout: Layout, origin: Origin) -> Geometry {
        Geometry::new(Size { width, height }, 1, 170)
            .unwrap()
            .with_wiring(Wiring { layout, origin })
    }

    /// The light (x, y) of each LED, in LED order.
    fn path(geometry: &Geometry) -> Vec<(usize, usize)> {
        let w = geometry.size().width as usize;
        (0..geometry.size().lights())
            .map(|led| {
                let light = geometry.light_of_led(led);
                (light % w, light / w)
            })
            .collect()
    }

    #[test]
    fn wirings_follow_their_layout_from_their_corner() {
        use Layout::*;
        use Origin::*;
        assert_eq!(
            path(&wired(3, 2, Rows, TopLeft)),
            [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]
        );
        assert_eq!(
            path(&wired(3, 2, SerpentineRows, TopLeft)),
            [(0, 0), (1, 0), (2, 0), (2, 1), (1, 1), (0, 1)]
        );
        assert_eq!(
            path(&wired(3, 2, SerpentineRows, BottomRight)),
            [(2, 1), (1, 1), (0, 1), (0, 0), (1, 0), (2, 0)]
        );
        assert_eq!(
            path(&wired(3, 2, Columns, TopRight)),
            [(2, 0), (2, 1), (1, 0), (1, 1), (0, 0), (0, 1)]
        );
        assert_eq!(
            path(&wired(3, 2, SerpentineColumns, BottomLeft)),
            [(0, 1), (0, 0), (1, 0), (1, 1), (2, 1), (2, 0)]
        );
    }

    #[test]
    fn every_wiring_is_a_permutation_and_led_of_light_inverts_it() {
        let layouts = [
            Layout::Rows,
            Layout::SerpentineRows,
            Layout::Columns,
            Layout::SerpentineColumns,
        ];
        let origins = [
            Origin::TopLeft,
            Origin::TopRight,
            Origin::BottomLeft,
            Origin::BottomRight,
        ];
        for (w, h) in [(1, 1), (8, 8), (32, 8), (7, 3), (1, 5)] {
            for layout in layouts {
                for origin in origins {
                    let geometry = wired(w, h, layout, origin);
                    let lights = geometry.size().lights();
                    let mut seen = vec![false; lights];
                    for led in 0..lights {
                        let light = geometry.light_of_led(led);
                        assert!(
                            !seen[light],
                            "{layout} {origin} {w}x{h}: light {light} twice"
                        );
                        seen[light] = true;
                        assert_eq!(geometry.led_of_light(light), led);
                    }
                }
            }
        }
    }

    #[test]
    fn a_column_serpentine_32_by_8_panel_matches_its_usual_pixel_mapper() {
        // the mapping usually given for these panels: led = x × 8 + y on even columns, x × 8 + 7 − y on odd ones
        let geometry = wired(32, 8, Layout::SerpentineColumns, Origin::TopLeft);
        for y in 0..8 {
            for x in 0..32 {
                let led = if x % 2 == 0 {
                    x * 8 + y
                } else {
                    x * 8 + (7 - y)
                };
                assert_eq!(geometry.led_of_light(y * 32 + x), led, "({x}, {y})");
            }
        }
    }

    #[test]
    fn frames_are_reordered_only_when_the_wiring_needs_it() {
        let frame: Vec<u8> = (0..18).collect(); // 3x2: lights 0..6, three bytes each
        let mut out = Vec::new();
        let straight = wired(3, 2, Layout::Rows, Origin::TopLeft);
        assert_eq!(
            straight.to_wiring_order(&frame, &mut out).as_ptr(),
            frame.as_ptr()
        );
        let serpentine = wired(3, 2, Layout::SerpentineRows, Origin::TopLeft);
        assert_eq!(
            serpentine.to_wiring_order(&frame, &mut out),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 15, 16, 17, 12, 13, 14, 9, 10, 11]
        );
    }

    #[test]
    fn layouts_and_origins_read_and_print() {
        for name in Layout::NAMES {
            assert_eq!(name.parse::<Layout>().unwrap().to_string(), name);
        }
        for name in Origin::NAMES {
            assert_eq!(name.parse::<Origin>().unwrap().to_string(), name);
        }
        assert!("zigzag".parse::<Layout>().is_err());
        assert!("left".parse::<Origin>().is_err());
    }

    #[test]
    fn layouts_out_of_range_are_refused() {
        let size = "192x64".parse().unwrap();
        assert_eq!(
            Geometry::new(size, 1, 0),
            Err(GeometryError::LightsPerUniverse(0))
        );
        assert_eq!(
            Geometry::new(size, 1, 171),
            Err(GeometryError::LightsPerUniverse(171))
        );
        assert_eq!(
            Geometry::new(size, 0, 170),
            Err(GeometryError::FirstUniverse(0))
        );
        assert!(Geometry::new(size, 63927, 170).is_ok()); // universes 63927..=63999
        assert_eq!(
            Geometry::new(size, 63928, 170),
            Err(GeometryError::TooManyUniverses {
                first: 63928,
                count: 73
            })
        );
    }
}
