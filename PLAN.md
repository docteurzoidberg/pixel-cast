# pixel-cast: plan

A command-line tool, written in Rust, that streams pictures to LED matrices over **E1.31 (sACN)**: test patterns,
still images, GIFs and videos. One self-contained binary per platform, meant to be run by other programs (scripts,
cron jobs, automation or agent servers) as much as by people.

pixel-cast is standalone and generic: it knows E1.31 and the geometry of a matrix, nothing about the receiver behind
them.

**Status (2026-11-26)**: step 1 under way: local repository, Cargo skeleton, MIT license and CI written; the GitHub
repository is not created yet.

## Decisions

| Topic | Decision |
|---|---|
| Name | `pixel-cast`: crate, repository `docteurzoidberg/pixel-cast` and command |
| Language | Rust |
| Repository | public, MIT license |
| Language of the repository | English everywhere: code, comments, messages, documentation, commits |
| First version | the command line: test patterns (one or several, animated, looping), still image, GIF and video (looping or once) |
| Receiver modes | not pixel-cast's business: it sends E1.31, whatever the receiver does with it |

## First version: the command line

```bash
pixel-cast -t wall pattern plasma                           # one animated pattern, looping
pixel-cast -t wall pattern grid universes chase --each 10s  # several patterns in turn, looping
pixel-cast -t wall image logo.png --duration 30s            # still image, resent while it is shown
pixel-cast -t wall gif animation.gif --loop                 # GIF, looping (--once: play it once)
pixel-cast -t wall video clip.mp4 --once --fit cover        # video, once (--loop: looping)
pixel-cast --host 192.0.2.50 --size 192x64 --universe 1 pattern gradient   # no target file needed
pixel-cast targets                                          # list the known targets
```

### Sources

| Source | Read by | Looping |
|---|---|---|
| `pattern <name>…` | computed: `universes`, `grid`, `gradient`, `plasma`, `chase`, `color <rgb>` | always; several patterns alternate every `--each` |
| `image <file>` | `image` crate (PNG, JPEG, BMP, WebP…) | n/a; `--duration` or Ctrl+C |
| `gif <file>` | `image` crate, with each frame's delay | `--loop` (default) or `--once` |
| `video <file>` | ffmpeg as a subprocess, raw RGB frames over a pipe | `--loop` or `--once` (default) |

The patterns are for setting up a display: `universes` gives each universe its own colour, `grid` shows orientation
and panel seams, `chase` lights one pixel after the other in index order (wiring), `color` tells two sources apart.

ffmpeg is only needed for `video`: the binary stays static, and a clear message says what to install when it is
missing.

### Targets

File `~/.config/pixel-cast/targets.toml` (per-platform path, `--config` for another one); command-line options
override the target.

```toml
[wall]
host = "192.0.2.21"        # the receiver (or a relay in front of it)
port = 5568
size = "192x64"
universe = 1               # first universe
lights_per_universe = 170
channels = "rgb"           # rgb, grb, rgbw…
layout = "rows"            # rows, serpentine; origin, flip, rotate
fps = 30
priority = 100
```

### Sending

- **Packets**: E1.31-2016 data packets; the picture is split into universes of `lights_per_universe` lights,
  index = y × width + x for `rows` (the usual layout of E1.31 matrix receivers).
- **Destination**: unicast (one or more addresses) or multicast (`239.255.<hi>.<lo>` per universe), configurable TTL
  and outgoing interface.
- **Header**: source name (`--source-name`, default `pixel-cast`), priority (`--priority`, default 100), a CID that
  stays the same on a given machine, one sequence counter per universe.
- **Pacing**: `--fps`, a fixed-deadline clock without drift (30 fps × 73 universes for 192 × 64 ≈ 2,200 packets/s).
  A still image is resent at a low rate (5 fps, say): sACN receivers drop a source after 2.5 s without packets.
- **Fitting**: `--fit contain | cover | stretch`, `--background` colour, a good downscaling filter (matrices are
  small).
- **End**: three packets with the "stream terminated" bit (E1.31 §6.2.6) at the end or on Ctrl+C. Optionally,
  `--blackout` sends a black frame first; without it, what the display shows next is up to the receiver.

### For other programs

- `--json`: one JSON line per second on stdout (frames, packets, actual frame rate, errors), then a summary at the
  end; human messages go to stderr.
- **Stable exit codes**: 0 normal end, 2 usage, 3 unreadable file, 4 network, 5 ffmpeg missing or failed.
- `--duration` so that a caller does not have to kill the process; SIGTERM handled like Ctrl+C (clean end).
- `--dry-run`: prepares everything and prints what would be sent, without the network.

## Architecture

```
src/
  main.rs        command line (clap), exit codes
  target.rs      TOML targets, merged with the options
  geometry.rs    size, wiring, channel order: RGB picture -> DMX values per universe
  sacn.rs        E1.31 packets (data, stream terminated), sequences, CID
  sender.rs      sockets (socket2): unicast, multicast, TTL, interface
  clock.rs       fixed-deadline pacing
  fit.rs         fitting, background
  source/
    pattern.rs   test patterns
    image.rs     still image
    gif.rs       animated GIF (per-frame delays)
    video.rs     ffmpeg subprocess
  report.rs      human or --json status
```

| Part | Choice | Why |
|---|---|---|
| Command line | `clap` | the usual one |
| sACN packets | hand-written | a simple format, no dependency; tested byte for byte |
| Network | `socket2` | multicast, TTL, interface |
| Images and GIFs | `image` | pure Rust decoding, GIF delays |
| Video | ffmpeg (subprocess) | no C library linked: a static binary |
| Configuration | `serde`, `toml`, `directories` | target file, per-platform paths |
| Shutdown | `ctrlc` (with SIGTERM) | clean end of stream |

**Tests**: packets byte for byte (layout of the standard, reference packets checked against an independent
implementation); universe splitting and wirings; fitting; a local UDP receiver that rebuilds the sent picture and
compares it with the source.

## Distribution

- GitHub releases built with `cargo-dist`: Linux x86_64 and ARM64 with musl (static: PC, server, Raspberry Pi),
  Windows, macOS.
- A minimal Docker image (the binary alone), and a variant with ffmpeg.
- `cargo install pixel-cast` once published on crates.io.

## Typical uses

- **By hand**: `pixel-cast -t wall gif animation.gif --loop`, Ctrl+C to stop.
- **From a script or a cron job**, on any machine of the network: one binary, one target file, `--duration` and the
  exit codes.
- **Driven by another program** (an automation or agent server): the program runs pixel-cast as a subprocess, reads
  `--json`, and keeps its own queue so that only one stream reaches a display at a time (see "Priority").
- **In a container**: unicast leaves a container fine; multicast from Docker Desktop is unreliable, so prefer unicast
  to the receiver there.

## Priority

Every E1.31 packet carries a priority from 0 to 200 (100 by default). A receiver that follows the standard keeps,
for each universe, only the sources with the highest priority, and merges or picks among them when they tie. So a
"notification" stream at 150 can take over a background stream at 100 without stopping it, and the background comes
back when the higher one ends.

Not every receiver honours it: some apply every packet, whatever its source, so two sources at once flicker.
pixel-cast sends a configurable priority (`--priority`, default 100); honouring it is the receiver's job, and a
caller that may start several streams should queue them itself.

## Steps

- [ ] 1. Create the `docteurzoidberg/pixel-cast` repository (public, MIT; Cargo, CI: fmt, clippy, tests).
- [ ] 2. Core: E1.31 packets, universe splitting, geometry and channels, TOML targets, unicast and multicast
      sending; byte-for-byte packet tests.
- [ ] 3. Sources: test patterns (`--each` to chain several), still image, GIF (`--loop` / `--once`), video through
      ffmpeg (`--loop` / `--once`), fitting.
- [ ] 4. Output and shutdown: `--json`, exit codes, `--duration`, clean end of stream, `--blackout`, `--dry-run`,
      `--priority`.
- [ ] 5. Real tests: a software receiver first, then real hardware.
- [ ] 6. Multi-platform releases (`cargo-dist`), Docker image, README.

## After the first version

- Raw frames on stdin (`pixel-cast -t wall raw --stdin`), for picture generators.
- Server mode: JSON commands (show, colour, stop) on stdin or over HTTP, one process for many streams.
- MCP server: pixel-cast as an agent's own tool.
- Scrolling text, clock, screen or window capture.
- Art-Net, sACN synchronization (sync universe), several displays tiled together.
