//! A video, decoded by ffmpeg in a subprocess: ffmpeg fits it to the matrix and resamples it to the stream's frame
//! rate, and writes raw RGB frames to a pipe.
//!
//! No video library is linked: the binary stays static, and ffmpeg is only needed for this source.

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{Repeat, Source, SourceError};
use crate::fit::{Color, Fit};
use crate::geometry::Size;

/// The ffmpeg program looked for on the PATH.
pub const FFMPEG: &str = "ffmpeg";

pub struct VideoSource {
    child: Child,
    stdout: ChildStdout,
    stderr: Option<JoinHandle<String>>,
    fps: f64,
    frame: Vec<u8>,
    frames_read: u64,
    ended: bool,
    description: String,
}

impl VideoSource {
    /// Starts `program` (normally [`FFMPEG`]) on `path`.
    pub fn open(
        program: &str,
        path: &Path,
        size: Size,
        fps: f64,
        fit: Fit,
        background: Color,
        repeat: Repeat,
    ) -> Result<Self, SourceError> {
        // a missing file is the caller's mistake, not ffmpeg's
        std::fs::metadata(path).map_err(|error| SourceError::File {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        let mut command = Command::new(program);
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        if repeat == Repeat::Loop {
            command.args(["-stream_loop", "-1"]);
        }
        command
            .arg("-i")
            .arg(path)
            .args(["-an", "-sn", "-vf", &filter(size, fps, fit, background)])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| match error.kind() {
            ErrorKind::NotFound => SourceError::FfmpegMissing {
                program: program.to_owned(),
            },
            _ => SourceError::Ffmpeg {
                message: error.to_string(),
            },
        })?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        // read stderr aside, so that a chatty ffmpeg never blocks on a full pipe
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        Ok(VideoSource {
            child,
            stdout,
            stderr: Some(stderr),
            fps,
            frame: vec![0; size.lights() * 3],
            frames_read: 0,
            ended: false,
            description: format!(
                "video {} (ffmpeg, {fit}, {})",
                path.display(),
                match repeat {
                    Repeat::Loop => "loop",
                    Repeat::Once => "once",
                }
            ),
        })
    }

    /// Reads the next frame; `false` at the end of the video.
    fn read_frame(&mut self) -> Result<bool, SourceError> {
        match self.stdout.read_exact(&mut self.frame) {
            Ok(()) => {
                self.frames_read += 1;
                Ok(true)
            }
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                self.ended = true;
                let status = self.child.wait().map_err(|e| SourceError::Ffmpeg {
                    message: e.to_string(),
                })?;
                let stderr = self
                    .stderr
                    .take()
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                let stderr = stderr.trim();
                if !status.success() {
                    let message = if stderr.is_empty() {
                        status.to_string()
                    } else {
                        stderr.to_owned()
                    };
                    return Err(SourceError::Ffmpeg { message });
                }
                if self.frames_read == 0 {
                    return Err(SourceError::Ffmpeg {
                        message: format!(
                            "no video frame{}",
                            if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            }
                        ),
                    });
                }
                Ok(false)
            }
            Err(error) => Err(SourceError::Ffmpeg {
                message: error.to_string(),
            }),
        }
    }
}

impl Source for VideoSource {
    /// Reads frames until the one due at `t` (ffmpeg gives them at the stream's frame rate); frames that are late
    /// are skipped, so the video keeps its speed.
    fn frame(&mut self, t: Duration) -> Result<Option<&[u8]>, SourceError> {
        let due = (t.as_secs_f64() * self.fps).floor() as u64 + 1;
        while !self.ended && self.frames_read < due {
            if !self.read_frame()? {
                return Ok(None);
            }
        }
        Ok(if self.ended { None } else { Some(&self.frame) })
    }

    fn describe(&self) -> String {
        self.description.clone()
    }
}

impl Drop for VideoSource {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The ffmpeg filter chain: frame rate, then the fit (scaling in RGB, so any size is fine).
pub fn filter(size: Size, fps: f64, fit: Fit, background: Color) -> String {
    let (w, h) = (size.width, size.height);
    let fitted = match fit {
        Fit::Contain => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x{}",
            background.hex()
        ),
        Fit::Cover => {
            format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h}")
        }
        Fit::Stretch => format!("scale={w}:{h}:flags=lanczos"),
    };
    format!("fps={fps},format=rgb24,{fitted},setsar=1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::gif::tests::write_test_gif;

    fn ffmpeg_present() -> bool {
        let present = Command::new(FFMPEG)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !present {
            eprintln!("ffmpeg is not installed: video test skipped");
        }
        present
    }

    #[test]
    fn filters_for_each_fit() {
        let size = Size {
            width: 192,
            height: 64,
        };
        assert_eq!(
            filter(size, 30.0, Fit::Contain, Color([1, 2, 3])),
            "fps=30,format=rgb24,scale=192:64:force_original_aspect_ratio=decrease:flags=lanczos,\
             pad=192:64:(ow-iw)/2:(oh-ih)/2:color=0x010203,setsar=1"
        );
        assert_eq!(
            filter(size, 29.97, Fit::Cover, Color::BLACK),
            "fps=29.97,format=rgb24,scale=192:64:force_original_aspect_ratio=increase:flags=lanczos,crop=192:64,setsar=1"
        );
        assert_eq!(
            filter(size, 25.0, Fit::Stretch, Color::BLACK),
            "fps=25,format=rgb24,scale=192:64:flags=lanczos,setsar=1"
        );
    }

    #[test]
    fn a_missing_ffmpeg_is_reported_as_such() {
        let dir =
            std::env::temp_dir().join(format!("pixel-cast-test-{}-noffmpeg", std::process::id()));
        let path = write_test_gif(&dir);
        let size = Size {
            width: 2,
            height: 2,
        };
        let result = VideoSource::open(
            "no-such-ffmpeg-program",
            &path,
            size,
            10.0,
            Fit::Contain,
            Color::BLACK,
            Repeat::Once,
        );
        assert!(matches!(result, Err(SourceError::FfmpegMissing { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_not_an_ffmpeg_error() {
        let size = Size {
            width: 2,
            height: 2,
        };
        let result = VideoSource::open(
            FFMPEG,
            Path::new("no/such/clip.mp4"),
            size,
            10.0,
            Fit::Contain,
            Color::BLACK,
            Repeat::Once,
        );
        assert!(matches!(result, Err(SourceError::File { .. })));
    }

    #[test]
    fn ffmpeg_decodes_fits_and_paces_frames() {
        if !ffmpeg_present() {
            return;
        }
        // the test GIF as a video: red 100 ms, green 200 ms, blue 100 ms; at 10 fps, 4 frames
        let dir =
            std::env::temp_dir().join(format!("pixel-cast-test-{}-video", std::process::id()));
        let path = write_test_gif(&dir);
        let size = Size {
            width: 4,
            height: 2,
        };
        let mut source = VideoSource::open(
            FFMPEG,
            &path,
            size,
            10.0,
            Fit::Contain,
            Color([0, 0, 9]),
            Repeat::Once,
        )
        .unwrap();
        let first = source.frame(Duration::ZERO).unwrap().unwrap().to_vec();
        assert_eq!(first.len(), 24);
        assert_eq!(&first[..3], &[0, 0, 9]); // a band on the left: 2x2 in 4x2
        assert!(first[3] > 200 && first[4] < 50, "red: {first:?}");
        let later = source
            .frame(Duration::from_millis(250))
            .unwrap()
            .unwrap()
            .to_vec();
        assert!(later[4] > 200 && later[3] < 50, "green: {later:?}");
        assert!(source.frame(Duration::from_secs(5)).unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
