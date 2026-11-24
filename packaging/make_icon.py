"""Draw the app icon (a spectrum that falls off a cliff) with NumPy only.

Writes realhires/assets/icon.png (512x512 RGBA).  PyInstaller turns it into
.ico/.icns at build time (with Pillow installed); the GUI uses it as the
window icon and the Linux installer as the menu icon.

Usage: python packaging/make_icon.py
"""

import os
import struct
import zlib

import numpy as np

SIZE = 512
SS = 4  # supersampling factor for anti-aliasing


def write_png(path: str, rgba: np.ndarray) -> None:
    h, w, _ = rgba.shape
    raw = b"".join(b"\x00" + rgba[y].tobytes() for y in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def main() -> None:
    n = SIZE * SS
    y, x = np.mgrid[0:n, 0:n].astype(np.float64) / n  # 0..1
    img = np.zeros((n, n, 4))

    # Rounded-square background with a vertical gradient.
    r, m = 0.19, 0.04
    cx = np.clip(x, m + r, 1 - m - r)
    cy = np.clip(y, m + r, 1 - m - r)
    inside = (x - cx) ** 2 + (y - cy) ** 2 <= r * r
    top, bottom = np.array([0.13, 0.20, 0.33]), np.array([0.05, 0.08, 0.15])
    bg = top[None, None, :] * (1 - y[..., None]) + bottom[None, None, :] * y[..., None]
    img[inside, :3] = bg[inside]
    img[inside, 3] = 1.0

    # Spectrum bars: gently falling level, then a brick-wall cliff.
    left, right, base = 0.16, 0.84, 0.80
    bars = 13
    cliff = 9  # bars after this index are "nothing above the cliff"
    width = (right - left) / bars
    for i in range(bars):
        x0 = left + i * width + width * 0.16
        x1 = left + (i + 1) * width - width * 0.16
        h = 0.52 - 0.022 * i if i < cliff else 0.035
        sel = inside & (x >= x0) & (x <= x1) & (y <= base) & (y >= base - h)
        t = (base - y[sel]) / 0.55
        if i < cliff:
            col = np.stack([0.20 + 0.25 * t, 0.85 - 0.1 * t, 0.80 + 0.15 * t], axis=-1)
        else:
            col = np.tile([0.45, 0.50, 0.60], (sel.sum(), 1))
        img[sel, :3] = col

    # Red dashed "cutoff" line at the cliff.
    xc = left + cliff * width
    dash = (np.floor(y * 22) % 2 == 0)
    sel = inside & (np.abs(x - xc) <= 0.0075) & (y >= 0.20) & (y <= base + 0.04) & dash
    img[sel, :3] = [0.93, 0.27, 0.27]

    # Baseline.
    sel = inside & (x >= left) & (x <= right) & (np.abs(y - (base + 0.02)) <= 0.008)
    img[sel, :3] = [0.75, 0.80, 0.88]

    # Downsample (box filter) for anti-aliasing, premultiplied alpha.
    pre = img.copy()
    pre[..., :3] *= pre[..., 3:4]
    small = pre.reshape(SIZE, SS, SIZE, SS, 4).mean(axis=(1, 3))
    alpha = small[..., 3:4]
    rgb = np.where(alpha > 0, small[..., :3] / np.maximum(alpha, 1e-9), 0)
    out = (np.concatenate([rgb, alpha], axis=-1) * 255).round().clip(0, 255).astype(np.uint8)

    dest = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "realhires", "assets", "icon.png")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    write_png(dest, out)
    print("wrote", os.path.abspath(dest))


if __name__ == "__main__":
    main()
