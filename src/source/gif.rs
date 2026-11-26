//! An animated GIF: every frame decoded and fitted once, then shown for its own delay.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage};

use super::{Repeat, Source, SourceError};
use crate::fit::{self, Color, Fit};
use crate::geometry::Size;

/// Delays this short are read as 100 ms, as browsers do: many GIFs say 0 and mean "the default".
const SHORTEST_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

pub struct GifSource {
    frames: Vec<Vec<u8>>,
    /// When each frame starts, from the start of the animation.
    starts: Vec<Duration>,
    total: Duration,
    repeat: Repeat,
}

impl GifSource {
    pub fn open(
        path: &Path,
        size: Size,
        fit: Fit,
        background: Color,
        repeat: Repeat,
    ) -> Result<Self, SourceError> {
        let error = |message: String| SourceError::File {
            path: path.to_owned(),
            message,
        };
        let file = File::open(path).map_err(|e| error(e.to_string()))?;
        let decoder = GifDecoder::new(BufReader::new(file)).map_err(|e| error(e.to_string()))?;
        let decoded = decoder
            .into_frames()
            .collect_frames()
            .map_err(|e| error(e.to_string()))?;
        if decoded.is_empty() {
            return Err(error("no frame".into()));
        }
        let mut frames = Vec::with_capacity(decoded.len());
        let mut starts = Vec::with_capacity(decoded.len());
        let mut total = Duration::ZERO;
        for frame in decoded {
            let delay = Duration::from(frame.delay());
            let delay = if delay < SHORTEST_DELAY {
                DEFAULT_DELAY
            } else {
                delay
            };
            starts.push(total);
            total += delay;
            frames.push(fit::fit(
                &DynamicImage::ImageRgba8(frame.into_buffer()),
                size,
                fit,
                background,
            ));
        }
        Ok(GifSource {
            frames,
            starts,
            total,
            repeat,
        })
    }

    /// The length of one pass through the animation.
    pub fn total(&self) -> Duration {
        self.total
    }

    /// The frame shown at `t`, or `None` past the end of a single pass.
    fn index_at(&self, t: Duration) -> Option<usize> {
        let t = match self.repeat {
            Repeat::Once if t >= self.total => return None,
            Repeat::Once => t,
            Repeat::Loop => Duration::from_nanos((t.as_nanos() % self.total.as_nanos()) as u64),
        };
        // the last frame that starts at or before t
        Some(self.starts.partition_point(|&start| start <= t) - 1)
    }
}

impl Source for GifSource {
    fn frame(&mut self, t: Duration) -> Result<Option<&[u8]>, SourceError> {
        Ok(self.index_at(t).map(|i| self.frames[i].as_slice()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat as GifRepeat};
    use image::{Delay, Frame, Rgba, RgbaImage};
    use std::path::PathBuf;

    /// A 2x2 GIF: red for 100 ms, green for 200 ms, blue with a delay of 0 (read as 100 ms).
    pub(crate) fn write_test_gif(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("test.gif");
        let mut encoder = GifEncoder::new(File::create(&path).unwrap());
        encoder.set_repeat(GifRepeat::Infinite).unwrap();
        let frames = [
            ([255, 0, 0, 255], 100),
            ([0, 255, 0, 255], 200),
            ([0, 0, 255, 255], 0),
        ]
        .map(|(color, ms)| {
            Frame::from_parts(
                RgbaImage::from_pixel(2, 2, Rgba(color)),
                0,
                0,
                Delay::from_numer_denom_ms(ms, 1),
            )
        });
        encoder.encode_frames(frames).unwrap();
        path
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pixel-cast-test-{}-{name}", std::process::id()))
    }

    fn red_of(source: &mut GifSource, ms: u64) -> Option<u8> {
        source
            .frame(Duration::from_millis(ms))
            .unwrap()
            .map(|frame| frame[0])
    }

    fn green_of(source: &mut GifSource, ms: u64) -> Option<u8> {
        source
            .frame(Duration::from_millis(ms))
            .unwrap()
            .map(|frame| frame[1])
    }

    #[test]
    fn frames_follow_their_delays_once() {
        let dir = scratch("gif-once");
        let path = write_test_gif(&dir);
        let size = Size {
            width: 2,
            height: 2,
        };
        let mut source =
            GifSource::open(&path, size, Fit::Contain, Color::BLACK, Repeat::Once).unwrap();
        assert_eq!(source.total(), Duration::from_millis(400));
        assert_eq!(red_of(&mut source, 0), Some(255));
        assert_eq!(red_of(&mut source, 99), Some(255));
        assert_eq!(green_of(&mut source, 100), Some(255));
        assert_eq!(green_of(&mut source, 299), Some(255));
        assert_eq!(
            source.frame(Duration::from_millis(350)).unwrap().unwrap()[2],
            255
        );
        assert_eq!(red_of(&mut source, 400), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_looping_gif_starts_again() {
        let dir = scratch("gif-loop");
        let path = write_test_gif(&dir);
        let size = Size {
            width: 2,
            height: 2,
        };
        let mut source =
            GifSource::open(&path, size, Fit::Contain, Color::BLACK, Repeat::Loop).unwrap();
        assert_eq!(red_of(&mut source, 400), Some(255));
        assert_eq!(green_of(&mut source, 4_150), Some(255));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
