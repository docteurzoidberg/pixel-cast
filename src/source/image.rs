//! A still image (PNG, JPEG, BMP, WebP, GIF's first frame…), fitted once and shown until the stream ends.

use std::path::Path;
use std::time::Duration;

use image::{DynamicImage, ImageReader};

use super::{Source, SourceError};
use crate::fit::{self, Color, Fit};
use crate::geometry::Size;

pub struct ImageSource {
    frame: Vec<u8>,
}

impl ImageSource {
    pub fn open(path: &Path, size: Size, fit: Fit, background: Color) -> Result<Self, SourceError> {
        let picture = decode(path)?;
        Ok(ImageSource {
            frame: fit::fit(&picture, size, fit, background),
        })
    }
}

impl Source for ImageSource {
    fn frame(&mut self, _t: Duration) -> Result<Option<&[u8]>, SourceError> {
        Ok(Some(&self.frame))
    }
}

/// Reads a picture file, its format guessed from its content.
pub fn decode(path: &Path) -> Result<DynamicImage, SourceError> {
    let error = |message: String| SourceError::File {
        path: path.to_owned(),
        message,
    };
    ImageReader::open(path)
        .map_err(|e| error(e.to_string()))?
        .with_guessed_format()
        .map_err(|e| error(e.to_string()))?
        .decode()
        .map_err(|e| error(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn a_png_is_fitted_once() {
        let dir =
            std::env::temp_dir().join(format!("pixel-cast-test-{}-image", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("picture.png");
        RgbImage::from_pixel(20, 10, Rgb([0, 200, 0]))
            .save(&path)
            .unwrap();

        let size = Size {
            width: 4,
            height: 4,
        };
        let mut source = ImageSource::open(&path, size, Fit::Contain, Color([0, 0, 50])).unwrap();
        let frame = source
            .frame(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .to_vec();
        assert_eq!(frame.len(), 48);
        assert_eq!(&frame[..3], &[0, 0, 50]); // top band
        assert_eq!(&frame[4 * 3 + 3..4 * 3 + 6], &[0, 200, 0]); // second row: the picture
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_files_are_reported() {
        let missing = Path::new("no/such/picture.png");
        assert!(matches!(decode(missing), Err(SourceError::File { .. })));
        let dir =
            std::env::temp_dir().join(format!("pixel-cast-test-{}-garbage", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let garbage = dir.join("garbage.png");
        std::fs::write(&garbage, b"not a picture").unwrap();
        assert!(matches!(decode(&garbage), Err(SourceError::File { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
