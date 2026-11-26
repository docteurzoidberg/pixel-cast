//! Fitting a picture of any size to the matrix.

use std::fmt;
use std::str::FromStr;

use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};

use crate::geometry::Size;

/// How a picture of another shape fills the matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// The whole picture, centred, with background bands where the shapes differ.
    #[default]
    Contain,
    /// The whole matrix, centred, cropping what sticks out.
    Cover,
    /// The whole picture over the whole matrix, distorted.
    Stretch,
}

impl Fit {
    pub const NAMES: [&str; 3] = ["contain", "cover", "stretch"];
}

impl FromStr for Fit {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "contain" => Ok(Fit::Contain),
            "cover" => Ok(Fit::Cover),
            "stretch" => Ok(Fit::Stretch),
            _ => Err(format!("unknown fit {text:?}: {}", Fit::NAMES.join(", "))),
        }
    }
}

impl fmt::Display for Fit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Fit::Contain => "contain",
            Fit::Cover => "cover",
            Fit::Stretch => "stretch",
        })
    }
}

/// An RGB colour, written `rrggbb` or `#rrggbb`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub const BLACK: Color = Color([0, 0, 0]);

    /// `rrggbb`, as ffmpeg's filters want it (`0xrrggbb`).
    pub fn hex(&self) -> String {
        let [r, g, b] = self.0;
        format!("{r:02x}{g:02x}{b:02x}")
    }
}

impl FromStr for Color {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.strip_prefix('#').unwrap_or(text);
        let bad = || format!("a colour is rrggbb, such as ff8000: {text:?}");
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let n = u32::from_str_radix(digits, 16).map_err(|_| bad())?;
        Ok(Color([(n >> 16) as u8, (n >> 8) as u8, n as u8]))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// The size a `source` picture is scaled to before it is centred on `target` (cropped or padded).
pub fn scaled_size(source: Size, target: Size, fit: Fit) -> Size {
    if fit == Fit::Stretch {
        return target;
    }
    let (sw, sh) = (f64::from(source.width), f64::from(source.height));
    let (tw, th) = (f64::from(target.width), f64::from(target.height));
    let scale = match fit {
        Fit::Contain => (tw / sw).min(th / sh),
        _ => (tw / sw).max(th / sh),
    };
    let side = |length: f64, bound: u32| -> u32 {
        let length = (length * scale).round().max(1.0) as u32;
        // rounding must not undo the fit: contain stays inside, cover covers
        match fit {
            Fit::Contain => length.min(bound),
            _ => length.max(bound),
        }
    };
    Size {
        width: side(sw, target.width),
        height: side(sh, target.height),
    }
}

/// `picture` fitted to `size`, over `background` where it is transparent or does not reach: RGB, rows from the
/// top-left corner.
///
/// Downscaling uses a Lanczos filter (detail survives on small matrices); upscaling keeps pixels square (pixel art
/// stays sharp).
pub fn fit(picture: &DynamicImage, size: Size, fit: Fit, background: Color) -> Vec<u8> {
    let rgba = picture.to_rgba8();
    let source = Size {
        width: rgba.width().max(1),
        height: rgba.height().max(1),
    };
    let scaled = scaled_size(source, size, fit);
    let resized = if scaled == source {
        rgba
    } else {
        let filter = if scaled.width < source.width || scaled.height < source.height {
            FilterType::Lanczos3
        } else {
            FilterType::Nearest
        };
        imageops::resize(&rgba, scaled.width, scaled.height, filter)
    };
    compose(&resized, size, background)
}

/// `picture` centred on a `size` canvas of `background` (cropped where larger), flattened to RGB.
fn compose(picture: &RgbaImage, size: Size, background: Color) -> Vec<u8> {
    let mut out = Vec::with_capacity(size.lights() * 3);
    // offset of the picture on the canvas; negative when the picture is larger (cover)
    let dx = (i64::from(size.width) - i64::from(picture.width())) / 2;
    let dy = (i64::from(size.height) - i64::from(picture.height())) / 2;
    for y in 0..i64::from(size.height) {
        for x in 0..i64::from(size.width) {
            let (px, py) = (x - dx, y - dy);
            let inside = px >= 0
                && py >= 0
                && px < i64::from(picture.width())
                && py < i64::from(picture.height());
            if inside {
                let [r, g, b, a] = picture.get_pixel(px as u32, py as u32).0;
                for (channel, bg) in [r, g, b].into_iter().zip(background.0) {
                    out.push(blend(channel, bg, a));
                }
            } else {
                out.extend_from_slice(&background.0);
            }
        }
    }
    out
}

fn blend(foreground: u8, background: u8, alpha: u8) -> u8 {
    let (f, b, a) = (
        u32::from(foreground),
        u32::from(background),
        u32::from(alpha),
    );
    ((f * a + b * (255 - a) + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    fn picture(width: u32, height: u32, color: [u8; 4]) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba(color)))
    }

    fn pixel(frame: &[u8], size: Size, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * size.width + x) * 3) as usize;
        frame[i..i + 3].try_into().unwrap()
    }

    #[test]
    fn scaled_sizes() {
        let wide = size(1920, 1080);
        let matrix = size(192, 64);
        assert_eq!(scaled_size(wide, matrix, Fit::Contain), size(114, 64));
        assert_eq!(scaled_size(wide, matrix, Fit::Cover), size(192, 108));
        assert_eq!(scaled_size(wide, matrix, Fit::Stretch), matrix);
        assert_eq!(
            scaled_size(size(8, 8), size(32, 8), Fit::Contain),
            size(8, 8)
        );
        assert_eq!(
            scaled_size(size(8, 8), size(32, 8), Fit::Cover),
            size(32, 32)
        );
        assert_eq!(
            scaled_size(size(1, 1000), size(16, 16), Fit::Contain),
            size(1, 16)
        );
    }

    #[test]
    fn contain_pads_with_the_background() {
        let red = picture(4, 2, [255, 0, 0, 255]);
        let matrix = size(4, 4);
        let frame = fit(&red, matrix, Fit::Contain, Color([0, 0, 9]));
        assert_eq!(frame.len(), 48);
        assert_eq!(pixel(&frame, matrix, 0, 0), [0, 0, 9]);
        assert_eq!(pixel(&frame, matrix, 0, 1), [255, 0, 0]);
        assert_eq!(pixel(&frame, matrix, 3, 2), [255, 0, 0]);
        assert_eq!(pixel(&frame, matrix, 3, 3), [0, 0, 9]);
    }

    #[test]
    fn cover_crops_the_centre() {
        // left half red, right half green, 4x2 on a 2x2 matrix: the centre column of each colour remains
        let mut wide = RgbaImage::from_pixel(4, 2, Rgba([255, 0, 0, 255]));
        for y in 0..2 {
            for x in 2..4 {
                wide.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        let matrix = size(2, 2);
        let frame = fit(
            &DynamicImage::ImageRgba8(wide),
            matrix,
            Fit::Cover,
            Color::BLACK,
        );
        assert_eq!(pixel(&frame, matrix, 0, 0), [255, 0, 0]);
        assert_eq!(pixel(&frame, matrix, 1, 1), [0, 255, 0]);
    }

    #[test]
    fn upscaling_keeps_pixels_square() {
        let mut checker = RgbaImage::new(2, 2);
        for (x, y, p) in checker.enumerate_pixels_mut() {
            *p = if (x + y) % 2 == 0 {
                Rgba([255; 4])
            } else {
                Rgba([0, 0, 0, 255])
            };
        }
        let matrix = size(4, 4);
        let frame = fit(
            &DynamicImage::ImageRgba8(checker),
            matrix,
            Fit::Stretch,
            Color::BLACK,
        );
        assert_eq!(pixel(&frame, matrix, 0, 0), [255; 3]);
        assert_eq!(pixel(&frame, matrix, 1, 1), [255; 3]);
        assert_eq!(pixel(&frame, matrix, 2, 1), [0; 3]);
    }

    #[test]
    fn transparency_shows_the_background() {
        let matrix = size(1, 1);
        let clear = fit(
            &picture(1, 1, [255, 255, 255, 0]),
            matrix,
            Fit::Contain,
            Color([10, 20, 30]),
        );
        assert_eq!(clear, [10, 20, 30]);
        let half = fit(
            &picture(1, 1, [255, 255, 255, 128]),
            matrix,
            Fit::Contain,
            Color::BLACK,
        );
        assert_eq!(half, [128, 128, 128]);
    }

    #[test]
    fn colours_and_fits_read_and_print() {
        assert_eq!("ff8000".parse(), Ok(Color([255, 128, 0])));
        assert_eq!("#00FF7f".parse(), Ok(Color([0, 255, 127])));
        assert!("fff".parse::<Color>().is_err());
        assert!("gg0000".parse::<Color>().is_err());
        assert_eq!(Color([1, 2, 3]).to_string(), "010203");
        assert_eq!("cover".parse(), Ok(Fit::Cover));
        assert!("fill".parse::<Fit>().is_err());
    }
}
