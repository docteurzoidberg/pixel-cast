//! The binary end to end: a stream to a loopback receiver, exit codes, the target file.

use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

const CID: &str = "0a0b0c0d-0000-4000-8000-000000000001";

fn pixel_cast(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pixel-cast"));
    command.args(args);
    command
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixel-cast-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs a stream to a loopback receiver; returns the process output and the option byte of every packet received.
fn stream(args: &[&str]) -> (Output, Vec<u8>) {
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let port = receiver.local_addr().unwrap().port().to_string();
    let mut full = vec!["--host", "127.0.0.1", "--port", &port, "--cid", CID];
    full.extend_from_slice(args);
    let output = pixel_cast(&full).output().unwrap();
    let mut options = Vec::new();
    let mut buffer = [0u8; 1500];
    while let Ok((len, _)) = receiver.recv_from(&mut buffer) {
        assert!(len >= 126);
        assert_eq!(
            &buffer[22..38],
            &[
                0x0a, 0x0b, 0x0c, 0x0d, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1
            ]
        );
        options.push(buffer[112]);
    }
    (output, options)
}

#[test]
fn a_pattern_streams_for_its_duration_then_ends_cleanly() {
    let (output, options) = stream(&[
        "--size",
        "8x8",
        "--fps",
        "20",
        "--duration",
        "500ms",
        "pattern",
        "plasma",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = options.iter().filter(|&&o| o == 0).count();
    let ends = options.iter().filter(|&&o| o == 0x40).count();
    // 10 ticks in 0.5 s at 20 fps; a slow machine may skip a few
    assert!(
        (5..=11).contains(&data),
        "{data} data packets in 0.5 s at 20 fps"
    );
    assert_eq!(ends, 3);
    assert!(output.stdout.is_empty(), "stdout stays for machine output");
}

#[test]
fn a_gif_played_once_ends_by_itself() {
    let dir = scratch("gif");
    let path = dir.join("one.gif");
    {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame, Rgba, RgbaImage};
        let mut encoder = GifEncoder::new(std::fs::File::create(&path).unwrap());
        let frames = [[255, 0, 0, 255], [0, 0, 255, 255]].map(|c| {
            Frame::from_parts(
                RgbaImage::from_pixel(4, 4, Rgba(c)),
                0,
                0,
                Delay::from_numer_denom_ms(150, 1),
            )
        });
        encoder.encode_frames(frames).unwrap();
    }
    let (output, options) = stream(&[
        "--size",
        "4x4",
        "--fps",
        "20",
        "gif",
        path.to_str().unwrap(),
        "--once",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(options.iter().filter(|&&o| o == 0x40).count(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exit_codes_tell_failures_apart() {
    let code = |args: &[&str]| pixel_cast(args).output().unwrap().status.code();
    let base = [
        "--host",
        "127.0.0.1",
        "--size",
        "8x8",
        "--cid",
        CID,
        "--duration",
        "100ms",
    ];
    let with = |more: &[&'static str]| [&base[..], more].concat();
    assert_eq!(code(&with(&["pattern", "nonsense"])), Some(2));
    assert_eq!(
        code(&["--size", "8x8", "--cid", CID, "pattern", "grid"]),
        Some(2)
    ); // no destination
    assert_eq!(code(&with(&["image", "no/such/picture.png"])), Some(3));
    assert_eq!(code(&with(&["video", "no/such/clip.mp4"])), Some(3));
    assert_eq!(
        code(&[
            "--host", "::1", "--size", "8x8", "--cid", CID, "pattern", "grid"
        ]),
        Some(4)
    );
}

#[test]
fn targets_come_from_the_file_and_options_override_them() {
    let dir = scratch("targets");
    let config = dir.join("targets.toml");
    std::fs::write(
        &config,
        "[wall]\nhost = \"192.0.2.21\"\nsize = \"192x64\"\n\n[broken]\nsize = \"8x8\"\n",
    )
    .unwrap();
    let output = pixel_cast(&["--config", config.to_str().unwrap(), "targets"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout).unwrap();
    assert!(
        listing.contains("wall: 192.0.2.21, port 5568; 192x64 in universes 1-73"),
        "{listing}"
    );
    assert!(
        listing.contains("broken: invalid: no destination"),
        "{listing}"
    );

    // -t wall, sent to a loopback receiver instead of its host, at another size
    let (output, options) = stream(&[
        "--config",
        config.to_str().unwrap(),
        "-t",
        "wall",
        "--size",
        "4x4",
        "--duration",
        "200ms",
        "pattern",
        "color",
        "ff0000",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(options.contains(&0x40));

    let unknown = pixel_cast(&[
        "--config",
        config.to_str().unwrap(),
        "-t",
        "nope",
        "pattern",
        "grid",
    ])
    .output()
    .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("known targets: broken, wall"));
    std::fs::remove_dir_all(&dir).unwrap();
}
