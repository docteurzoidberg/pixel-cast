//! Size of a matrix and its split into universes.
//!
//! A picture is RGB, three bytes per light, lights in rows from the top-left corner (index = y × width + x). Each
//! universe carries `lights_per_universe` lights; the last one carries what is left. Mapping lights to the
//! physical wiring is the receiver's job.

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

    pub fn size(&self) -> Size {
        self.size
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

    /// Splits an RGB picture into `(universe, DMX values)`, the values borrowed from `frame`.
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

/// A size or universe layout that cannot be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryError {
    Size(String),
    LightsPerUniverse(u16),
    FirstUniverse(u16),
    TooManyUniverses { first: u16, count: usize },
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
