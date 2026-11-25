"""Make the app icon from the user's picture of a maki roll (2.1).

Reads packaging/icon-source.png (the user's picture: the roll on a checkerboard and
white background that is painted into the picture, not transparent), cuts the roll
out along its dark seaweed edge, centres it with a small margin and writes
realhires/assets/icon.png (512x512 RGBA, transparent around the roll).  PyInstaller
turns that into .ico/.icns at build time (with Pillow installed), the Rust build
embeds it (rust/windows_resources.rs, the window icon in crates/gui/src/main.rs, an
.icns on macOS), the apps use it as the window icon and the Linux installers as the
menu icon.

The cut: every pixel darker than DARK is seaweed, and the roll is the convex hull of
those pixels (its outline is convex), sampled 4x4 per pixel for a smooth edge without
a fringe of the background.  FFmpeg decodes the picture and scales the result
(Lanczos).

Usage: python packaging/make_icon.py
"""

import os
import subprocess
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
from realhires.probe import tool  # noqa: E402

SOURCE = os.path.join(HERE, "icon-source.png")
DEST = os.path.join(HERE, "..", "realhires", "assets", "icon.png")
SIZE = 512
DARK = 120     # luminance below this is the seaweed rim (the background is 220-255)
MARGIN = 0.05  # free space on each side, as a share of the icon


def read_rgba(path: str) -> np.ndarray:
    probe = subprocess.run([tool("ffprobe"), "-v", "error", "-select_streams", "v:0", "-show_entries",
                            "stream=width,height", "-of", "csv=p=0:s=x", path],
                           capture_output=True, text=True, check=True).stdout.strip()
    w, h = (int(v) for v in probe.split("x"))
    raw = subprocess.run([tool("ffmpeg"), "-v", "error", "-i", path, "-f", "rawvideo", "-pix_fmt", "rgba", "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.uint8).reshape(h, w, 4).copy()


def convex_hull(points: np.ndarray) -> np.ndarray:
    """The convex hull of (x, y) points, counter-clockwise (Andrew's monotone chain)."""
    pts = sorted(set(map(tuple, points.tolist())))

    def half(seq):
        out = []
        for p in seq:
            while len(out) >= 2 and ((out[-1][0] - out[-2][0]) * (p[1] - out[-2][1])
                                     - (out[-1][1] - out[-2][1]) * (p[0] - out[-2][0])) <= 0:
                out.pop()
            out.append(p)
        return out

    lower, upper = half(pts), half(reversed(pts))
    return np.array(lower[:-1] + upper[:-1], dtype=np.float64)


def roll_mask(rgba: np.ndarray) -> np.ndarray:
    """The share of each pixel inside the roll's outline (0..1, anti-aliased): the convex
    hull of the seaweed's dark pixels, sampled 4x4 per pixel."""
    lum = rgba[..., :3].astype(np.float64) @ np.array([0.299, 0.587, 0.114])
    ys, xs = np.nonzero(lum < DARK)
    hull = convex_hull(np.stack([xs, ys], axis=1))
    h, w = lum.shape
    y0, y1, x0, x1 = max(ys.min() - 2, 0), min(ys.max() + 3, h), max(xs.min() - 2, 0), min(xs.max() + 3, w)
    gy, gx = np.mgrid[y0:y1, x0:x1].astype(np.float64)
    cover = np.zeros(gy.shape)
    nxt = np.roll(hull, -1, axis=0)
    for oy in (-0.375, -0.125, 0.125, 0.375):
        for ox in (-0.375, -0.125, 0.125, 0.375):
            px, py = gx + ox, gy + oy
            inside = np.ones(gy.shape, bool)
            for (ax, ay), (bx, by) in zip(hull, nxt):  # left of every counter-clockwise edge
                inside &= (bx - ax) * (py - ay) - (by - ay) * (px - ax) >= 0
            cover += inside
    mask = np.zeros((h, w))
    mask[y0:y1, x0:x1] = cover / 16.0
    return mask


def main() -> None:
    rgba = read_rgba(SOURCE)
    mask = roll_mask(rgba)
    ys, xs = np.nonzero(mask > 0)
    side = int(round(max(ys.max() - ys.min(), xs.max() - xs.min()) / (1 - 2 * MARGIN)))
    cy, cx = (ys.min() + ys.max()) / 2, (xs.min() + xs.max()) / 2
    top, left = int(round(cy - side / 2)), int(round(cx - side / 2))
    out = np.zeros((side, side, 4), np.uint8)
    h, w = mask.shape
    y0, x0 = max(top, 0), max(left, 0)
    y1, x1 = min(top + side, h), min(left + side, w)
    out[y0 - top:y1 - top, x0 - left:x1 - left, :3] = rgba[y0:y1, x0:x1, :3]
    out[y0 - top:y1 - top, x0 - left:x1 - left, 3] = np.round(mask[y0:y1, x0:x1] * 255).astype(np.uint8)
    subprocess.run([tool("ffmpeg"), "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba",
                    "-s", f"{side}x{side}", "-i", "-", "-vf", f"scale={SIZE}:{SIZE}:flags=lanczos",
                    "-frames:v", "1", "-pix_fmt", "rgba", os.path.abspath(DEST)],
                   input=out.tobytes(), check=True)
    print(f"wrote {os.path.abspath(DEST)} ({SIZE}x{SIZE}) from a {side}x{side} cut of {os.path.basename(SOURCE)}")


if __name__ == "__main__":
    main()
