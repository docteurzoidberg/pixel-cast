#!/usr/bin/env python3
"""Generates the sample files of this folder: a still image, an animated GIF and a video for each small matrix size.

Needs Pillow and ffmpeg (with libx264). Run from anywhere:

    python3 samples/make_samples.py

Everything is drawn from scratch here, so the files carry no third-party content.
"""

import colorsys
import subprocess
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
SIZES = [(8, 8), (16, 16), (32, 8)]

GIF_FRAMES = 40  # 2 s at 50 ms a frame
VIDEO_FPS = 30
VIDEO_SECONDS = 3


def hsv(h, s, v):
    r, g, b = colorsys.hsv_to_rgb(h % 1.0, s, v)
    return (round(r * 255), round(g * 255), round(b * 255))


def triangle(phase):
    """0 -> 1 -> 0 as phase goes 0 -> 0.5 -> 1."""
    phase %= 1.0
    return 2 * phase if phase < 0.5 else 2 - 2 * phase


def still(w, h):
    """Diagonal rainbow, one colour per corner (top-left red, top-right green, bottom-left blue, bottom-right white)
    and a transparent block in the centre (shows --background)."""
    image = Image.new("RGBA", (w, h))
    for y in range(h):
        for x in range(w):
            # hues from orange to violet, dimmed: the pure corner colours stand out
            image.putpixel((x, y), hsv(0.1 + 0.75 * (x / w + y / h) / 2, 0.8, 0.5) + (255,))
    for (x, y), color in {
        (0, 0): (255, 0, 0),
        (w - 1, 0): (0, 255, 0),
        (0, h - 1): (0, 0, 255),
        (w - 1, h - 1): (255, 255, 255),
    }.items():
        image.putpixel((x, y), color + (255,))
    side = max(2, min(w, h) // 4)
    for y in range((h - side) // 2, (h + side) // 2):
        for x in range((w - side) // 2, (w + side) // 2):
            image.putpixel((x, y), (0, 0, 0, 0))
    return image


def gif_frames(w, h):
    """A white dot bouncing off the edges with a short trail, over a dim background slowly changing hue."""
    frames = []
    for i in range(GIF_FRAMES):
        background = hsv(i / GIF_FRAMES, 1.0, 0.15)
        frame = Image.new("RGB", (w, h), background)
        for age, level in ((2, 70), (1, 140), (0, 255)):
            j = i - age
            x = round(triangle(j / GIF_FRAMES) * (w - 1))
            y = round(triangle(2 * j / GIF_FRAMES) * (h - 1))
            frame.putpixel((x, y), (level, level, level))
        frames.append(frame)
    return frames


def video_frame(w, h, i, count):
    """A band of colour sweeping along the long side, its hue turning; the bottom row fills from the left as the
    video plays (shows where playback is, and its end)."""
    frame = Image.new("RGB", (w, h))
    t = i / count
    long_side = max(w, h)
    band = triangle(2 * t) * (long_side - 1)
    for y in range(h):
        for x in range(w):
            along = x if w >= h else y
            distance = abs(along - band)
            level = max(0.0, 1.0 - distance / 2.5)
            frame.putpixel((x, y), hsv(t + along / long_side / 2, 1.0, 0.1 + 0.9 * level))
    filled = round(t * w)
    for x in range(filled):
        frame.putpixel((x, h - 1), (255, 255, 255))
    return frame


def write_video(w, h, path):
    count = VIDEO_FPS * VIDEO_SECONDS
    command = [
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
        "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{w}x{h}", "-r", str(VIDEO_FPS), "-i", "-",
        # 4:4:4 keeps every pixel's colour at these sizes (4:2:0 would share colour between 2x2 pixels)
        "-c:v", "libx264", "-preset", "veryslow", "-crf", "12", "-pix_fmt", "yuv444p",
        "-movflags", "+faststart", "-map_metadata", "-1", "-fflags", "+bitexact",
        str(path),
    ]
    frames = b"".join(video_frame(w, h, i, count).tobytes() for i in range(count))
    subprocess.run(command, input=frames, check=True)


def main():
    for w, h in SIZES:
        name = f"test-{w}x{h}"
        still(w, h).save(HERE / f"{name}.png", optimize=True)
        frames = gif_frames(w, h)
        frames[0].save(
            HERE / f"{name}.gif", save_all=True, append_images=frames[1:], duration=50, loop=0, optimize=False
        )
        write_video(w, h, HERE / f"{name}.mp4")
        print(f"{name}: png, gif, mp4")


if __name__ == "__main__":
    main()
