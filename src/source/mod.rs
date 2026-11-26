//! Sources of pictures: test patterns, still images, animated GIFs and videos.
//!
//! Every source gives pictures already fitted to the matrix (RGB, rows from the top-left corner), by time since the
//! start of the stream.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub mod gif;
pub mod image;
pub mod pattern;
pub mod video;

/// Pictures over time.
pub trait Source {
    /// The picture to show at `t` from the start, or `None` once the source has ended.
    ///
    /// Calls come with `t` increasing.
    fn frame(&mut self, t: Duration) -> Result<Option<&[u8]>, SourceError>;

    /// What the source shows, for reports: `pattern grid, chase (10 s each)`, `gif a.gif (12 frames, 1.2 s, loop)`.
    fn describe(&self) -> String;
}

/// Whether a finite source starts again at its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Loop,
    Once,
}

/// A source that cannot give pictures.
#[derive(Debug)]
pub enum SourceError {
    /// The file cannot be opened or decoded.
    File { path: PathBuf, message: String },
    /// ffmpeg is not installed, or not where it was looked for.
    FfmpegMissing { program: String },
    /// ffmpeg ran but failed.
    Ffmpeg { message: String },
    /// A pattern list or argument that does not make sense.
    Pattern(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::File { path, message } => write!(f, "{}: {message}", path.display()),
            SourceError::FfmpegMissing { program } => write!(
                f,
                "videos need ffmpeg, and {program:?} was not found: install ffmpeg (https://ffmpeg.org/download.html) \
                 and make sure it is on the PATH"
            ),
            SourceError::Ffmpeg { message } => write!(f, "ffmpeg failed: {message}"),
            SourceError::Pattern(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SourceError {}
