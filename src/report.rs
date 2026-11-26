//! Status of a stream: human lines on stderr, or JSON lines on stdout (`--json`) for other programs.
//!
//! JSON events, one object per line:
//!
//! - `start`: the target and the source;
//! - `progress`, about once a second: totals so far and the frame rate of the last second;
//! - `end`: why the stream ended, and the totals;
//! - `error`: a failure, with the exit code the process ends with;
//! - `dry_run`: what `--dry-run` found (instead of the others).

use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::time::Duration;

use crate::sender::Sent;
use crate::target::{Destination, Target};

/// Interval of progress reports.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// Why a stream ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// `--duration` was reached.
    Duration,
    /// The source ended (a GIF or a video played once).
    SourceEnded,
    /// Ctrl+C or SIGTERM.
    Signal,
}

impl EndReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            EndReason::Duration => "duration",
            EndReason::SourceEnded => "source_ended",
            EndReason::Signal => "signal",
        }
    }
}

/// Totals of a stream so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub elapsed: Duration,
    /// Frames sent.
    pub frames: u64,
    /// Clock ticks given up because the previous frame ended too late.
    pub skipped: u64,
    pub sent: Sent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Lines on stderr; `live` rewrites a status line in place (stderr is a terminal).
    Human { live: bool },
    /// JSON lines on stdout.
    Json,
}

impl Mode {
    pub fn new(json: bool) -> Self {
        if json {
            Mode::Json
        } else {
            Mode::Human {
                live: std::io::stderr().is_terminal(),
            }
        }
    }
}

pub struct Reporter {
    mode: Mode,
    last: Totals,
    last_report: Duration,
    status_shown: bool,
}

impl Reporter {
    pub fn new(mode: Mode) -> Self {
        Reporter {
            mode,
            last: Totals::default(),
            last_report: Duration::ZERO,
            status_shown: false,
        }
    }

    pub fn start(&mut self, target: &Target, source: &str) {
        match self.mode {
            Mode::Json => {
                let mut event = Json::event("start");
                target_fields(&mut event, target);
                event.str("source", source);
                out(event);
            }
            Mode::Human { .. } => eprintln!("pixel-cast: {source} -> {}", describe(target)),
        }
    }

    /// Reports progress when a second has passed since the last report; `error` is the last send failure since then.
    pub fn progress(&mut self, totals: &Totals, error: Option<&str>) {
        if totals.elapsed < self.last_report + PROGRESS_INTERVAL {
            return;
        }
        let span = (totals.elapsed - self.last_report).as_secs_f64();
        let fps = (totals.frames - self.last.frames) as f64 / span;
        let errors = totals.sent.errors - self.last.sent.errors;
        match self.mode {
            Mode::Json => {
                let mut event = Json::event("progress");
                totals_fields(&mut event, totals);
                event.num("fps", round2(fps));
                if let Some(error) = error {
                    event.str("error", error);
                }
                out(event);
            }
            Mode::Human { live } => {
                if errors > 0 {
                    self.clear_status();
                    eprintln!(
                        "pixel-cast: warning: {errors} datagrams not sent in the last second: {}",
                        error.unwrap_or("unknown error")
                    );
                }
                if live {
                    eprint!(
                        "\r{:.0} s: {} frames, {fps:.1} fps, {} packets\x1b[K",
                        totals.elapsed.as_secs_f64(),
                        totals.frames,
                        totals.sent.packets
                    );
                    self.status_shown = true;
                }
            }
        }
        self.last = *totals;
        self.last_report = totals.elapsed;
    }

    pub fn end(&mut self, totals: &Totals, reason: EndReason, error: Option<&str>) {
        let seconds = totals.elapsed.as_secs_f64();
        let fps = if seconds > 0.0 {
            totals.frames as f64 / seconds
        } else {
            0.0
        };
        match self.mode {
            Mode::Json => {
                let mut event = Json::event("end");
                event.str("reason", reason.as_str());
                totals_fields(&mut event, totals);
                event.num("fps", round2(fps));
                if let Some(error) = error {
                    event.str("error", error);
                }
                out(event);
            }
            Mode::Human { .. } => {
                self.clear_status();
                let why = match reason {
                    EndReason::Duration => "duration reached",
                    EndReason::SourceEnded => "end of the source",
                    EndReason::Signal => "stopped",
                };
                let mut line = format!(
                    "pixel-cast: {why} after {seconds:.1} s: {} frames ({fps:.1} fps), {} packets",
                    totals.frames, totals.sent.packets
                );
                if totals.skipped > 0 {
                    let _ = write!(line, ", {} late frames skipped", totals.skipped);
                }
                if totals.sent.errors > 0 {
                    let _ = write!(line, ", {} datagrams not sent", totals.sent.errors);
                }
                eprintln!("{line}");
            }
        }
    }

    /// A failure: always a line on stderr, and an `error` event with `--json`.
    pub fn error(&mut self, code: u8, message: &str) {
        self.clear_status();
        if self.mode == Mode::Json {
            let mut event = Json::event("error");
            event.int("code", code.into());
            event.str("message", message);
            out(event);
        }
        eprintln!("pixel-cast: {message}");
    }

    pub fn warning(&mut self, message: &str) {
        self.clear_status();
        eprintln!("pixel-cast: warning: {message}");
    }

    fn clear_status(&mut self) {
        if self.status_shown {
            eprint!("\r\x1b[K");
            self.status_shown = false;
        }
    }
}

/// What `--dry-run` found: everything is ready, nothing was sent.
pub fn dry_run(json: bool, target: &Target, source: &str) {
    let geometry = target.geometry;
    let per_frame = geometry.universe_count();
    let values = geometry.frame_len();
    let fps = target.fps;
    let most = per_frame as f64 * fps;
    if json {
        let mut event = Json::event("dry_run");
        target_fields(&mut event, target);
        event.str("source", source);
        event.int("packets_per_frame", per_frame as u64);
        event.int("bytes_per_frame", values as u64);
        event.num("packets_per_second_max", round2(most));
        out(event);
        return;
    }
    let first = geometry.first_universe() as usize;
    let last = first + per_frame - 1;
    let per_universe = geometry.lights_per_universe() as usize;
    let in_last = geometry.size().lights() - (per_frame - 1) * per_universe;
    let mut text = String::new();
    let _ = writeln!(text, "destination: {}", destination(&target.destination));
    let _ = writeln!(text, "size: {}", geometry.size());
    let _ = writeln!(
        text,
        "wiring: {}, first LED {}",
        geometry.wiring().layout,
        geometry.wiring().origin
    );
    let _ = writeln!(
        text,
        "universes: {first}-{last} ({per_universe} lights each, {in_last} in the last)"
    );
    let _ = writeln!(text, "frame rate: {fps} fps");
    let _ = writeln!(text, "source name: {}", target.source.name());
    let _ = writeln!(text, "priority: {}", target.source.priority());
    let _ = writeln!(text, "cid: {}", target.source.cid());
    let _ = writeln!(text, "source: {source}");
    let _ = writeln!(
        text,
        "packets per frame: {per_frame} ({values} bytes of values)"
    );
    let _ = writeln!(
        text,
        "packets per second: at most {most:.0} (an unchanged universe: about one a second)"
    );
    print!("{text}");
}

fn out(event: Json) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", event.finish());
    let _ = stdout.flush();
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn destination(destination: &Destination) -> String {
    match destination {
        Destination::Unicast { hosts, port } => format!("{}, port {port}", hosts.join(", ")),
        Destination::Multicast {
            port,
            ttl,
            interface,
        } => format!(
            "multicast, port {port}, TTL {ttl}{}",
            interface
                .map(|i| format!(", interface {i}"))
                .unwrap_or_default()
        ),
    }
}

/// One line on a target, for humans.
pub fn describe(target: &Target) -> String {
    let geometry = target.geometry;
    let first = geometry.first_universe();
    let last = first as usize + geometry.universe_count() - 1;
    let universes = if last == first as usize {
        format!("universe {first}")
    } else {
        format!("universes {first}-{last}")
    };
    let wiring = geometry.wiring();
    let wiring = if wiring == Default::default() {
        String::new()
    } else {
        format!(", {} from {}", wiring.layout, wiring.origin)
    };
    format!(
        "{}; {}{wiring} in {universes} of {} lights; {} fps; priority {}",
        destination(&target.destination),
        geometry.size(),
        geometry.lights_per_universe(),
        target.fps,
        target.source.priority()
    )
}

fn target_fields(event: &mut Json, target: &Target) {
    match &target.destination {
        Destination::Unicast { hosts, port } => {
            event.str_list("hosts", hosts);
            event.int("port", (*port).into());
        }
        Destination::Multicast {
            port,
            ttl,
            interface,
        } => {
            event.bool("multicast", true);
            event.int("port", (*port).into());
            event.int("ttl", (*ttl).into());
            if let Some(interface) = interface {
                event.str("interface", &interface.to_string());
            }
        }
    }
    let geometry = target.geometry;
    event.str("size", &geometry.size().to_string());
    event.str("layout", &geometry.wiring().layout.to_string());
    event.str("origin", &geometry.wiring().origin.to_string());
    event.int("first_universe", geometry.first_universe().into());
    event.int("universes", geometry.universe_count() as u64);
    event.int("lights_per_universe", geometry.lights_per_universe().into());
    event.num("fps", target.fps);
    event.int("priority", target.source.priority().into());
    event.str("source_name", target.source.name());
    event.str("cid", &target.source.cid().to_string());
}

fn totals_fields(event: &mut Json, totals: &Totals) {
    event.num("elapsed", round2(totals.elapsed.as_secs_f64()));
    event.int("frames", totals.frames);
    event.int("packets", totals.sent.packets as u64);
    event.int("datagrams", totals.sent.datagrams as u64);
    event.int("suppressed", totals.sent.suppressed as u64);
    event.int("skipped", totals.skipped);
    event.int("errors", totals.sent.errors as u64);
}

/// A flat JSON object, written by hand: a few fields of plain types do not need a serializer.
pub struct Json {
    text: String,
}

impl Json {
    pub fn event(name: &str) -> Self {
        let mut json = Json {
            text: String::from("{"),
        };
        json.str("event", name);
        json
    }

    fn key(&mut self, key: &str) {
        if self.text.len() > 1 {
            self.text.push(',');
        }
        push_string(&mut self.text, key);
        self.text.push(':');
    }

    pub fn str(&mut self, key: &str, value: &str) {
        self.key(key);
        push_string(&mut self.text, value);
    }

    pub fn str_list(&mut self, key: &str, values: &[String]) {
        self.key(key);
        self.text.push('[');
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                self.text.push(',');
            }
            push_string(&mut self.text, value);
        }
        self.text.push(']');
    }

    pub fn int(&mut self, key: &str, value: u64) {
        self.key(key);
        let _ = write!(self.text, "{value}");
    }

    /// A number; JSON has no infinity or NaN, so those become `null`.
    pub fn num(&mut self, key: &str, value: f64) {
        self.key(key);
        if value.is_finite() {
            let _ = write!(self.text, "{value}");
        } else {
            self.text.push_str("null");
        }
    }

    pub fn bool(&mut self, key: &str, value: bool) {
        self.key(key);
        self.text.push_str(if value { "true" } else { "false" });
    }

    pub fn finish(mut self) -> String {
        self.text.push('}');
        self.text
    }
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_objects_escape_their_strings() {
        let mut event = Json::event("test");
        event.str("text", "a \"quote\", a \\ and a\nnew line\u{1}");
        event.int("n", 42);
        event.num("x", 29.97);
        event.num("nan", f64::NAN);
        event.bool("yes", true);
        event.str_list("hosts", &["a".into(), "b".into()]);
        assert_eq!(
            event.finish(),
            r#"{"event":"test","text":"a \"quote\", a \\ and a\nnew line\u0001","n":42,"x":29.97,"nan":null,"yes":true,"hosts":["a","b"]}"#
        );
    }

    #[test]
    fn progress_comes_once_a_second() {
        let mut reporter = Reporter::new(Mode::Human { live: false });
        let mut totals = Totals::default();
        for ms in (0..3000).step_by(100) {
            totals.elapsed = Duration::from_millis(ms);
            totals.frames += 3;
            reporter.progress(&totals, None);
        }
        assert_eq!(reporter.last_report, Duration::from_secs(2));
        assert_eq!(reporter.last.frames, 63);
    }
}
