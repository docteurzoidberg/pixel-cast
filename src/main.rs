//! pixel-cast: stream test patterns, images, GIFs and videos to LED matrices over E1.31 (sACN).

use std::fmt;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pixel_cast::clock::Clock;
use pixel_cast::fit::{Color, Fit};
use pixel_cast::report::{self, EndReason, Mode, Reporter, Totals};
use pixel_cast::sacn::Cid;
use pixel_cast::sender::{SendError, Stream};
use pixel_cast::source::gif::GifSource;
use pixel_cast::source::image::ImageSource;
use pixel_cast::source::pattern::{Pattern, PatternSource};
use pixel_cast::source::video::{FFMPEG, VideoSource};
use pixel_cast::source::{Repeat, Source, SourceError};
use pixel_cast::target::{self, Hosts, Target, TargetError, TargetSpec, Targets};

#[derive(Parser, Debug)]
#[command(
    name = "pixel-cast",
    version,
    about = "Stream test patterns, images, GIFs and videos to LED matrices over E1.31 (sACN)",
    after_help = "Exit codes: 0 normal end, 2 usage, 3 unreadable file, 4 network, 5 ffmpeg missing or failed."
)]
struct Cli {
    /// Target from the target file (see `pixel-cast targets`); the options below override it.
    #[arg(short, long, global = true)]
    target: Option<String>,
    /// Target file [default: per platform, ~/.config/pixel-cast/targets.toml on Linux].
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Unicast destination: host name or IPv4 address; several separated by commas.
    #[arg(long, global = true, value_delimiter = ',', value_name = "HOST")]
    host: Vec<String>,
    /// Send each universe to its multicast group (239.255.<high>.<low>) instead of a host.
    #[arg(long, global = true)]
    multicast: bool,
    /// UDP port [default: 5568].
    #[arg(long, global = true)]
    port: Option<u16>,
    /// Multicast time to live [default: 1].
    #[arg(long, global = true)]
    ttl: Option<u32>,
    /// IPv4 address of the interface multicast leaves from.
    #[arg(long, global = true, value_name = "ADDRESS")]
    interface: Option<Ipv4Addr>,

    /// Matrix size, such as 192x64.
    #[arg(long, global = true, value_name = "WxH")]
    size: Option<String>,
    /// First universe [default: 1].
    #[arg(long, global = true)]
    universe: Option<u16>,
    /// Lights per universe, three channels each [default: 170].
    #[arg(long, global = true, value_name = "N")]
    lights_per_universe: Option<u16>,
    /// How LEDs run: rows, serpentine-rows, columns or serpentine-columns [default: rows].
    #[arg(long, global = true)]
    layout: Option<String>,
    /// Corner of the first LED: top-left, top-right, bottom-left or bottom-right [default: top-left].
    #[arg(long, global = true, value_name = "CORNER")]
    origin: Option<String>,
    /// Frames per second [default: 30].
    #[arg(long, global = true)]
    fps: Option<f64>,
    /// Priority, 0 to 200 [default: 100].
    #[arg(long, global = true)]
    priority: Option<u8>,
    /// Source name shown by receivers [default: pixel-cast].
    #[arg(long, global = true, value_name = "NAME")]
    source_name: Option<String>,
    /// Component identifier [default: one kept per machine].
    #[arg(long, global = true, value_name = "UUID")]
    cid: Option<Cid>,

    /// How pictures of another shape fill the matrix: contain, cover or stretch.
    #[arg(long, global = true, default_value = "contain")]
    fit: Fit,
    /// Colour around and behind pictures (rrggbb).
    #[arg(long, global = true, default_value = "000000", value_name = "RRGGBB")]
    background: Color,
    /// Stop after this long, such as 30s, 500ms or 2m [default: when the source ends].
    #[arg(long, global = true, value_parser = parse_duration)]
    duration: Option<Duration>,
    /// Send a black frame before the end of the stream.
    #[arg(long, global = true)]
    blackout: bool,
    /// Prepare everything (target, files, ffmpeg) and print what would be sent, without sending.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Status as JSON lines on stdout: start, progress once a second, end or error.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Test patterns: universes, grid, gradient, plasma, chase, color <rrggbb>; several take turns.
    Pattern {
        /// Pattern names, in turn.
        #[arg(required = true, num_args = 1.., value_name = "PATTERN")]
        patterns: Vec<String>,
        /// Time each pattern is shown when there are several.
        #[arg(long, default_value = "10s", value_parser = parse_duration)]
        each: Duration,
    },
    /// A still image (PNG, JPEG, BMP, WebP, GIF), until --duration or Ctrl+C.
    Image { file: PathBuf },
    /// An animated GIF, looping unless --once.
    Gif {
        file: PathBuf,
        #[command(flatten)]
        repeat: RepeatArgs,
    },
    /// A video through ffmpeg, once unless --loop.
    Video {
        file: PathBuf,
        #[command(flatten)]
        repeat: RepeatArgs,
    },
    /// List the targets of the target file.
    Targets,
}

#[derive(Args, Debug)]
#[group(multiple = false)]
struct RepeatArgs {
    /// Start again at the end.
    #[arg(long = "loop")]
    repeat: bool,
    /// Play it once.
    #[arg(long)]
    once: bool,
}

impl RepeatArgs {
    fn or(&self, default: Repeat) -> Repeat {
        if self.repeat {
            Repeat::Loop
        } else if self.once {
            Repeat::Once
        } else {
            default
        }
    }
}

/// `30s`, `1.5s`, `500ms`, `2m`, `1h`, or seconds alone.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number.trim().parse().map_err(|_| {
        format!("a duration is a number and a unit (ms, s, m, h), such as 30s: {text:?}")
    })?;
    let seconds = match unit {
        "" | "s" => value,
        "ms" => value / 1000.0,
        "m" | "min" => value * 60.0,
        "h" => value * 3600.0,
        _ => return Err(format!("unknown unit {unit:?}: use ms, s, m or h")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|_| format!("not a duration: {text:?}"))
}

/// A failure, with the exit code it ends the process with.
#[derive(Debug)]
enum Failure {
    Usage(String),
    File(String),
    Network(String),
    Ffmpeg(String),
}

impl Failure {
    fn code(&self) -> u8 {
        match self {
            Failure::Usage(_) => 2,
            Failure::File(_) => 3,
            Failure::Network(_) => 4,
            Failure::Ffmpeg(_) => 5,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Failure::Usage(m) | Failure::File(m) | Failure::Network(m) | Failure::Ffmpeg(m)) =
            self;
        f.write_str(m)
    }
}

impl From<TargetError> for Failure {
    fn from(error: TargetError) -> Self {
        match error {
            TargetError::Read { .. } => Failure::File(error.to_string()),
            _ => Failure::Usage(error.to_string()),
        }
    }
}

impl From<SourceError> for Failure {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::File { .. } => Failure::File(error.to_string()),
            SourceError::FfmpegMissing { .. } | SourceError::Ffmpeg { .. } => {
                Failure::Ffmpeg(error.to_string())
            }
            SourceError::Pattern(_) => Failure::Usage(error.to_string()),
        }
    }
}

impl From<SendError> for Failure {
    fn from(error: SendError) -> Self {
        Failure::Network(error.to_string())
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut reporter = Reporter::new(Mode::new(cli.json));
    match run(&cli, &mut reporter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            reporter.error(failure.code(), &failure.to_string());
            ExitCode::from(failure.code())
        }
    }
}

fn run(cli: &Cli, reporter: &mut Reporter) -> Result<(), Failure> {
    if let Command::Targets = cli.command {
        return list_targets(cli);
    }
    let target = resolve_target(cli, reporter)?;
    let mut source = open_source(cli, &target)?;

    if cli.dry_run {
        // the first frame proves the source decodes (for a video: that ffmpeg runs)
        if source.frame(Duration::ZERO)?.is_none() {
            return Err(Failure::File(format!("{}: no frame", source.describe())));
        }
        report::dry_run(cli.json, &target, &source.describe());
        return Ok(());
    }

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        if let Err(error) = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst)) {
            reporter.warning(&format!("Ctrl+C will not end the stream cleanly: {error}"));
        }
    }

    let mut stream = Stream::open(&target)?;
    reporter.start(&target, &source.describe());
    let mut clock = Clock::new(target.fps, Instant::now());
    let mut totals = Totals::default();
    let mut last_error: Option<String> = None;
    let reason = loop {
        if stop.load(Ordering::SeqCst) {
            break EndReason::Signal;
        }
        let tick = clock.wait();
        totals.skipped += tick.skipped;
        if stop.load(Ordering::SeqCst) {
            break EndReason::Signal;
        }
        if cli.duration.is_some_and(|limit| tick.at >= limit) {
            break EndReason::Duration;
        }
        let Some(frame) = source.frame(tick.at)? else {
            break EndReason::SourceEnded;
        };
        totals.sent += stream.send_frame(frame, Instant::now());
        totals.frames += 1;
        totals.elapsed = clock.start().elapsed();
        let error = stream.take_error().map(|e| e.to_string());
        if error.is_some() {
            last_error = error.clone();
        }
        reporter.progress(&totals, error.as_deref());
    };
    if cli.blackout {
        let black = vec![0; target.geometry.frame_len()];
        totals.sent += stream.send_frame(&black, Instant::now());
    }
    totals.sent += stream.terminate();
    totals.elapsed = clock.start().elapsed();
    if let Some(error) = stream.take_error() {
        last_error = Some(error.to_string());
    }
    reporter.end(&totals, reason, last_error.as_deref());
    if totals.sent.datagrams == 0 && totals.sent.errors > 0 {
        return Err(Failure::Network(format!(
            "nothing could be sent: {}",
            last_error.unwrap_or_default()
        )));
    }
    Ok(())
}

fn open_source(cli: &Cli, target: &Target) -> Result<Box<dyn Source>, Failure> {
    let size = target.geometry.size();
    Ok(match &cli.command {
        Command::Pattern { patterns, each } => Box::new(PatternSource::new(
            Pattern::parse_list(patterns)?,
            *each,
            target.geometry,
        )?),
        Command::Image { file } => {
            Box::new(ImageSource::open(file, size, cli.fit, cli.background)?)
        }
        Command::Gif { file, repeat } => Box::new(GifSource::open(
            file,
            size,
            cli.fit,
            cli.background,
            repeat.or(Repeat::Loop),
        )?),
        Command::Video { file, repeat } => Box::new(VideoSource::open(
            FFMPEG,
            file,
            size,
            target.fps,
            cli.fit,
            cli.background,
            repeat.or(Repeat::Once),
        )?),
        Command::Targets => unreachable!("handled before"),
    })
}

fn resolve_target(cli: &Cli, reporter: &mut Reporter) -> Result<Target, Failure> {
    let options = TargetSpec {
        host: (!cli.host.is_empty()).then(|| Hosts::Many(cli.host.clone())),
        multicast: cli.multicast.then_some(true),
        port: cli.port,
        ttl: cli.ttl,
        interface: cli.interface,
        size: cli.size.clone(),
        universe: cli.universe,
        lights_per_universe: cli.lights_per_universe,
        layout: cli.layout.clone(),
        origin: cli.origin.clone(),
        fps: cli.fps,
        priority: cli.priority,
        source_name: cli.source_name.clone(),
    };
    let spec = match &cli.target {
        Some(name) => {
            let targets = load_targets(cli)?;
            let mut file = targets.get(name)?.clone();
            // a host on the command line replaces multicast from the file, and the other way round
            if options.host.is_some() {
                file.multicast = None;
            }
            if options.multicast.is_some() {
                file.host = None;
            }
            options.or(file)
        }
        None => options,
    };
    Ok(spec.resolve(machine_cid(cli, reporter))?)
}

fn load_targets(cli: &Cli) -> Result<Targets, Failure> {
    let (path, required) = match &cli.config {
        Some(path) => (path.clone(), true),
        None => (
            target::default_targets_path().ok_or_else(|| {
                Failure::Usage("no configuration folder on this system: use --config".into())
            })?,
            false,
        ),
    };
    Ok(Targets::load(&path, required)?)
}

/// `--cid`, or the one kept for this machine; a random one, with a warning, when it cannot be kept.
fn machine_cid(cli: &Cli, reporter: &mut Reporter) -> Cid {
    if let Some(cid) = cli.cid {
        return cid;
    }
    match target::default_cid_path().map(|path| target::load_or_create_cid(&path)) {
        Some(Ok(cid)) => cid,
        Some(Err(error)) => {
            reporter.warning(&format!("{error}; using a new CID for this run"));
            Cid::random()
        }
        None => Cid::random(),
    }
}

fn list_targets(cli: &Cli) -> Result<(), Failure> {
    let targets = load_targets(cli)?;
    if targets.names().next().is_none() {
        let path = cli.config.clone().or_else(target::default_targets_path);
        eprintln!(
            "pixel-cast: no targets defined{}",
            path.map(|p| format!(" in {}", p.display()))
                .unwrap_or_default()
        );
        return Ok(());
    }
    for (name, spec) in targets.iter() {
        match spec.resolve(Cid([0; 16])) {
            Ok(target) => println!("{name}: {}", report::describe(&target)),
            Err(error) => println!("{name}: invalid: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30s"), Ok(Duration::from_secs(30)));
        assert_eq!(parse_duration("1.5s"), Ok(Duration::from_millis(1500)));
        assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_duration("2m"), Ok(Duration::from_secs(120)));
        assert_eq!(parse_duration("1h"), Ok(Duration::from_secs(3600)));
        assert_eq!(parse_duration("4"), Ok(Duration::from_secs(4)));
        for bad in ["", "s", "10 parsecs", "-1s", "1e400s"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_command_line_parses() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        let cli = Cli::try_parse_from([
            "pixel-cast",
            "-t",
            "wall",
            "pattern",
            "grid",
            "color",
            "ff0000",
            "--each",
            "5s",
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Command::Pattern { ref patterns, each } if patterns.len() == 3 && each == Duration::from_secs(5))
        );
        let cli = Cli::try_parse_from(["pixel-cast", "gif", "a.gif", "--once", "--fit", "cover"])
            .unwrap();
        assert!(
            matches!(cli.command, Command::Gif { ref repeat, .. } if repeat.or(Repeat::Loop) == Repeat::Once)
        );
        assert_eq!(cli.fit, Fit::Cover);
        assert!(Cli::try_parse_from(["pixel-cast", "video", "a.mp4", "--loop", "--once"]).is_err());
        let cli = Cli::try_parse_from([
            "pixel-cast",
            "--host",
            "192.0.2.1,192.0.2.2",
            "image",
            "a.png",
        ])
        .unwrap();
        assert_eq!(cli.host, ["192.0.2.1", "192.0.2.2"]);
    }
}
