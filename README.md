# pixel-cast

A command-line tool that streams test patterns, still images, GIFs and videos to LED matrices over
**E1.31 (sACN)**. One self-contained binary, meant to be driven by scripts and other programs as much as by people.

- Test patterns computed from the matrix's own geometry: universes, grid, gradient, plasma, chase, plain colour.
- Still images (PNG, JPEG, BMP, WebP), animated GIFs, and videos through ffmpeg; fitted to the matrix.
- Any size and universe layout; LEDs wired in rows or columns, straight or serpentine, from any corner.
- Unicast to one or more hosts, or multicast.
- Follows ANSI E1.31-2025: priorities, sequence numbers, keep-alive of unchanged data, universe discovery, clean
  end of stream.
- For other programs: JSON status lines, stable exit codes, `--duration`, `--dry-run`, SIGTERM handled.

**Status**: under development, no release yet: build it from source for now (below). See [`PLAN.md`](PLAN.md) for the
design and the roadmap.

## Install

From source, with a [Rust toolchain](https://rustup.rs) (1.85 or later):

```bash
cargo install --git https://github.com/docteurzoidberg/pixel-cast
```

Videos need [ffmpeg](https://ffmpeg.org/download.html) on the `PATH`; everything else works without it.

Once released, each [release](https://github.com/docteurzoidberg/pixel-cast/releases) carries ready-made binaries:
Linux x86_64 and ARM64 (static, no dependency: PC, server, Raspberry Pi), Windows x86_64, macOS ARM64 and x86_64.

### Docker

Two images, for amd64 and arm64: `ghcr.io/docteurzoidberg/pixel-cast` (the binary alone, about 2 MB) and
`ghcr.io/docteurzoidberg/pixel-cast:ffmpeg` (with ffmpeg, for videos).

```bash
docker run --rm ghcr.io/docteurzoidberg/pixel-cast --host 192.0.2.50 --size 16x16 --duration 30s pattern grid

# a target file in ./config/pixel-cast/targets.toml, a stable CID in the pixel-cast-data volume, videos from ./media
docker run --rm -v "$PWD/config:/config:ro" -v pixel-cast-data:/data -v "$PWD/media:/media:ro" \
  ghcr.io/docteurzoidberg/pixel-cast:ffmpeg -t wall video /media/clip.mp4
```

Unicast works from the default network; multicast needs `--network host` (Linux), and is unreliable from Docker
Desktop. `docker stop` sends SIGTERM, which ends the stream cleanly.

## Quick start

```bash
# a test pattern on a 16x16 matrix at 192.0.2.50, until Ctrl+C
pixel-cast --host 192.0.2.50 --size 16x16 pattern grid

# several patterns in turn, 10 s each
pixel-cast --host 192.0.2.50 --size 16x16 pattern grid universes chase --each 10s

# a still image for 30 s, an animated GIF once, a video looping
pixel-cast --host 192.0.2.50 --size 16x16 image logo.png --duration 30s
pixel-cast --host 192.0.2.50 --size 16x16 gif animation.gif --once
pixel-cast --host 192.0.2.50 --size 16x16 video clip.mp4 --loop --fit cover

# the same, with the matrix described once in the target file
pixel-cast -t wall pattern plasma
```

Ctrl+C ends a stream cleanly (see [Ending a stream](#ending-a-stream)).

## Sources

| Command | Plays |
|---|---|
| `pattern <name>…` | one or more test patterns, forever; several take turns every `--each` (default 10 s) |
| `image <file>` | a still image (PNG, JPEG, BMP, WebP, the first frame of a GIF) until `--duration` or Ctrl+C |
| `gif <file>` | an animated GIF with each frame's own delay; loops unless `--once` |
| `video <file>` | any video ffmpeg reads, resampled to the stream's frame rate; plays once unless `--loop` |

### Test patterns

Patterns are computed from the target's geometry (size, universes, wiring), so they work at any size, and they move
in seconds, not frames, so they look the same at any frame rate.

| Pattern | Shows | Checks |
|---|---|---|
| `universes` | each universe in its own colour, a white light running through every universe at once | universe boundaries, lights per universe |
| `grid` | an orange edge, a grid drawn from the centre, the centre in yellow, one colour per corner (top-left red, top-right green, bottom-left blue, bottom-right white), a white line sweeping left to right | orientation, wiring, cropping |
| `gradient` | hue across, brightness down | colour order, dynamic range |
| `plasma` | smooth waves of colour | frame rate, smoothness |
| `chase` | one light after the other in reading order | wiring: once right, it runs row by row from the top-left |
| `color <rrggbb>` | one colour everywhere | telling two sources apart, white balance |

```bash
pixel-cast -t wall pattern color ff0000
pixel-cast -t wall pattern grid color 00ff00 chase --each 5s
```

### Fitting pictures

Images, GIFs and videos of another size are scaled to the matrix:

| `--fit` | Result |
|---|---|
| `contain` (default) | the whole picture, centred, with bands of `--background` where the shapes differ |
| `cover` | the whole matrix, centred, cropping what sticks out |
| `stretch` | the whole picture over the whole matrix, distorted |

`--background <rrggbb>` (default black) also shows through transparent pixels. Shrinking uses a Lanczos filter;
enlarging keeps pixels square, so pixel art stays sharp.

## Targets

A target names a matrix and how to reach it. Targets live in `targets.toml`, in the platform's configuration folder
(`~/.config/pixel-cast/targets.toml` on Linux; `pixel-cast targets` says where it looks), or in the file given with
`--config`.

```toml
[wall]
host = "192.0.2.21"        # or several: ["192.0.2.21", "192.0.2.22"]; or multicast = true
port = 5568
size = "192x64"
universe = 1               # first universe
lights_per_universe = 170
layout = "rows"            # rows, serpentine-rows, columns, serpentine-columns
origin = "top-left"        # corner of the first LED
fps = 30
priority = 100
source_name = "pixel-cast"

[panel]
host = "192.0.2.30"
size = "32x8"
layout = "serpentine-columns"

[everywhere]
multicast = true
ttl = 4
interface = "192.0.2.1"    # IPv4 address of the interface multicast leaves from
size = "16x16"
```

Only a destination (`host` or `multicast`) and `size` are required. Unknown keys are refused, so that a typo does
not go unnoticed. Command-line options override the target, and work without one:

```bash
pixel-cast targets                                      # list the targets and what they resolve to
pixel-cast -t wall --priority 150 pattern color ff0000  # the wall target at a higher priority
pixel-cast --host 192.0.2.50 --size 8x8 pattern grid    # no target file needed
```

## Options

| Option | Default | |
|---|---|---|
| `-t`, `--target <name>` | | target from the target file |
| `--config <file>` | per platform | target file |
| `--host <host>[,<host>…]` | | unicast destinations: IPv4 addresses or host names |
| `--multicast` | | each universe to its group, 239.255.&lt;high&gt;.&lt;low&gt; |
| `--port <port>` | 5568 | UDP port |
| `--ttl <n>` | 1 | multicast time to live |
| `--interface <address>` | system's choice | IPv4 address of the interface multicast leaves from |
| `--size <W>x<H>` | | matrix size in lights, such as `192x64` |
| `--universe <n>` | 1 | first universe |
| `--lights-per-universe <n>` | 170 | lights per universe, three channels each (at most 170) |
| `--layout <layout>` | `rows` | how LEDs run: `rows`, `serpentine-rows`, `columns`, `serpentine-columns` |
| `--origin <corner>` | `top-left` | corner of the first LED: `top-left`, `top-right`, `bottom-left`, `bottom-right` |
| `--fps <n>` | 30 | frames per second |
| `--priority <n>` | 100 | E1.31 priority, 0 to 200 |
| `--source-name <name>` | `pixel-cast` | name receivers show for this source (63 bytes at most) |
| `--cid <uuid>` | one per machine | component identifier |
| `--fit <fit>` | `contain` | `contain`, `cover` or `stretch` |
| `--background <rrggbb>` | `000000` | colour around and behind pictures |
| `--duration <time>` | until the source ends | `500ms`, `30s`, `2m`, `1h` |
| `--blackout` | | send a black frame before the end |
| `--dry-run` | | prepare everything, print what would be sent, send nothing |
| `--json` | | status as JSON lines on stdout |

## Wiring

pixel-cast draws pictures in rows from the top-left corner, then puts them in the order the LEDs are wired before
splitting them into universes. Most receivers take DMX values in LED order with no mapping of their own, so the
wiring is described on the sender's side:

```
 rows                serpentine-rows      columns             serpentine-columns
 0 → 1 → 2 → 3       0 → 1 → 2 → 3        0   3   6   9       0   5 → 6   11
                                 ↓        ↓   ↓   ↓   ↓       ↓   ↑   ↓   ↑
 4 → 5 → 6 → 7       7 ← 6 ← 5 ← 4        1   4   7   10      1   4   7   10
                     ↓                    ↓   ↓   ↓   ↓       ↓   ↑   ↓   ↑
 8 → 9 → 10 → 11     8 → 9 → 10 → 11      2   5   8   11      2 → 3   8 → 9
```

`--origin` is the corner of LED 0, seen from the front; rows or columns start from there. The `grid` pattern shows
whether the wiring is right: corners in their colours, a straight sweeping line, the yellow centre in the middle.
Common 8x32 panels, for instance, are usually `serpentine-columns` from the top-left.

## How it streams

- **Universes**: each universe carries `lights_per_universe` LEDs in wiring order, three channels (R, G, B) each;
  the last one carries what is left. A 192x64 matrix at 170 lights per universe takes universes 1 to 73.
- **Pacing**: a fixed-deadline clock, so the frame rate does not drift; when a frame ends too late, missed ticks are
  skipped rather than sent in a burst.
- **Unchanged data**: as E1.31 §6.6.2 requires, a universe whose values do not change is sent three times, then
  once every 900 ms as a keep-alive, so a still image costs about one packet per universe per second, and receivers
  never drop the source (they do after 2.5 s of silence).
- **Universe discovery**: every 10 s, a packet lists the stream's universes (E1.31 §4.3), so that monitoring tools
  such as sACNView show the source; it goes to the discovery group 239.255.250.214 in multicast, to the same hosts
  in unicast. Receivers that do not use it ignore it.
- **Priorities**: every packet carries `--priority`. A receiver that follows the standard shows, for each universe,
  the source with the highest priority: a stream at 150 takes over one at 100 without stopping it, and the lower
  one is shown again when the higher one ends. Because a still background is only kept alive, it can take up to
  0.9 s to come back. Not every receiver honours priorities; with those, two sources at once flicker, so a program
  that may start several streams should queue them.
- **Send errors** (a network that drops for a while) do not stop the stream: they are counted and reported.

### Ending a stream

At the end (`--duration`, the end of a GIF or video played once, Ctrl+C or SIGTERM), pixel-cast sends three
packets per universe with the "stream terminated" bit (E1.31 §6.2.6): receivers drop the source at once instead
of waiting for a timeout. These packets carry the last values sent, so that receivers which ignore the bit keep
the last picture. With `--blackout`, a black frame goes out first, and the matrix ends dark either way.

## For other programs

- **Output**: human messages go to stderr; stdout carries only machine output (`--json` lines, the `--dry-run`
  report, `targets`).
- **`--duration`** ends the stream by itself; **SIGTERM** ends it cleanly, like Ctrl+C.
- **Exit codes**:

  | Code | Meaning |
  |---|---|
  | 0 | normal end, including Ctrl+C and SIGTERM |
  | 2 | usage: bad option, unknown target or pattern, invalid target file |
  | 3 | a file cannot be read: picture, GIF, video, target file |
  | 4 | network: no socket, a host that does not resolve, nothing could be sent |
  | 5 | ffmpeg missing or failed |

- **`--json`**: one JSON object per line on stdout: `start`, then `progress` about once a second, then `end`; or
  `error` (also written to stderr):

  ```json
  {"event":"start","hosts":["192.0.2.50"],"port":5568,"size":"16x16","layout":"rows","origin":"top-left","first_universe":1,"universes":2,"lights_per_universe":170,"fps":30,"priority":100,"source_name":"pixel-cast","cid":"7c9e6679-7425-40de-944b-e07fc1f90ae7","source":"pattern plasma"}
  {"event":"progress","elapsed":1,"frames":31,"packets":62,"datagrams":62,"suppressed":0,"skipped":0,"errors":0,"fps":30.99}
  {"event":"end","reason":"duration","elapsed":2.2,"frames":66,"packets":138,"datagrams":138,"suppressed":0,"skipped":0,"errors":0,"fps":30}
  {"event":"error","code":3,"message":"no/such/picture.png: No such file or directory (os error 2)"}
  ```

  `reason` is `duration`, `source_ended` or `signal`. `packets` counts E1.31 packets, `datagrams` UDP datagrams
  sent (packets × hosts), `suppressed` universes left out because unchanged, `skipped` late frames, `errors`
  datagrams the system refused; `progress` and `end` may carry the last `error` message. Multicast targets have
  `multicast`, `ttl` and `interface` instead of `hosts`.

- **`--dry-run`** prepares everything (target, files, ffmpeg, the first frame) and prints what would be sent,
  without sending; it fails with the same exit codes as a real run:

  ```
  $ pixel-cast --host 192.0.2.50 --size 16x16 --layout serpentine-rows --dry-run gif samples/test-16x16.gif
  destination: 192.0.2.50, port 5568
  size: 16x16
  wiring: serpentine-rows, first LED top-left
  universes: 1-2 (170 lights each, 86 in the last)
  frame rate: 30 fps
  source name: pixel-cast
  priority: 100
  cid: 7c9e6679-7425-40de-944b-e07fc1f90ae7
  source: gif samples/test-16x16.gif (contain): 40 frames, 2.00 s, loop
  packets per frame: 2 (768 bytes of values)
  packets per second: at most 60 (an unchanged universe: about one a second)
  ```

  With `--json`, the same as one `dry_run` event.

- **CID**: the component identifier stays the same on a machine (kept in `~/.local/share/pixel-cast/cid` on Linux),
  so receivers recognise the source across runs; give `--cid` to choose one.

## Sample files

[`samples/`](samples/) holds a still image, a GIF and a video for 8x8, 16x16 and 32x8 matrices, to try pixel-cast
on a small matrix without looking for files:

```bash
pixel-cast --host 192.0.2.50 --size 8x8 gif samples/test-8x8.gif
```

## The standard

pixel-cast implements ANSI E1.31-2025, published by ESTA, a free download from ESTA's Technical Standards Program;
see [`docs/standard/README.md`](docs/standard/README.md) for where to get it and the sections pixel-cast relies on.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests send nothing on the network: they stream to a receiver on the loopback interface, which rebuilds the
pictures. The video tests need ffmpeg and are skipped without it.

## License

[MIT](LICENSE)
