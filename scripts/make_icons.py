#!/usr/bin/env python3
"""Renders the MemManager app icon (stdlib only) and writes:

    windows/app/memmanager.ico     Windows exe / installer icon (PNG-compressed, 16-256 px)
    macos/Resources/AppIcon.icns   macOS bundle icon (16-1024 px)
    assets/icon.svg                vector master (same geometry)
    assets/icon-256.png            README image

The design: a blue tile with a white 270-degree gauge, about two-thirds full,
around a small centre dot -- the same ring as the tray / menu bar icon, so
the app and its status icon read as one. Every size is rendered natively
with signed-distance anti-aliasing; small sizes get thicker strokes.

    python3 scripts/make_icons.py
"""
import math
import os
import struct
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

TOP = (0x3B, 0xA4, 0xF7)  # gradient, top-left
BOTTOM = (0x0B, 0x5C, 0xD6)  # gradient, bottom-right
FILL = 0.64  # how full the gauge is
START, SWEEP = 225.0, 270.0  # degrees; clockwise from bottom-left


def geometry(size: int, mac: bool):
    """Tile margin, corner radius, ring radius/width, dot radius (unit square)."""
    margin = 0.098 if mac else 0.03  # macOS icons sit inside a ~10% safe area
    tile = 1 - 2 * margin
    small = size <= 32
    return {
        "margin": margin,
        "corner": tile * (0.225 if mac else 0.2),
        "r": tile * 0.30,
        "w": tile * (0.13 if small else 0.095),
        "dot": 0.0 if size <= 24 else tile * 0.065,
    }


def arc_dist(px, py, r, a0, sweep):
    """Distance from (px, py) to a clockwise arc of radius r from angle a0 (degrees)."""
    ang = math.degrees(math.atan2(py, px))
    # Clockwise offset from the start angle, in [0, 360).
    off = (a0 - ang) % 360.0
    if off <= sweep:
        return abs(math.hypot(px, py) - r)
    best = float("inf")
    for a in (a0, a0 - sweep):
        cx, cy = r * math.cos(math.radians(a)), r * math.sin(math.radians(a))
        best = min(best, math.hypot(px - cx, py - cy))
    return best


def rrect_dist(px, py, half, rad):
    qx, qy = abs(px) - half + rad, abs(py) - half + rad
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    return outside + min(max(qx, qy), 0.0) - rad


def cov(d_px):
    return min(1.0, max(0.0, 0.5 - d_px))


def render(size: int, mac: bool) -> bytes:
    g = geometry(size, mac)
    half = 0.5 - g["margin"]
    rows = []
    for j in range(size):
        row = bytearray([0])  # PNG filter: none
        for i in range(size):
            # Unit coordinates centred on the tile, y up.
            x = (i + 0.5) / size - 0.5
            y = 0.5 - (j + 0.5) / size
            tile = cov(rrect_dist(x, y, half, g["corner"]) * size)
            if tile <= 0:
                row += b"\0\0\0\0"
                continue
            t = min(1.0, max(0.0, ((0.5 - y) + (x + 0.5)) / 2))
            bg = [TOP[k] + (BOTTOM[k] - TOP[k]) * t for k in range(3)]
            track = cov((arc_dist(x, y, g["r"], START, SWEEP) - g["w"] / 2) * size)
            value = cov((arc_dist(x, y, g["r"], START, SWEEP * FILL) - g["w"] / 2) * size)
            dot = cov((math.hypot(x, y) - g["dot"]) * size) if g["dot"] else 0.0
            white = max(value, dot, track * 0.32)
            rgb = [bg[k] + (255 - bg[k]) * white for k in range(3)]
            row += bytes([round(rgb[0]), round(rgb[1]), round(rgb[2]), round(255 * tile)])
        rows.append(bytes(row))
    raw = b"".join(rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def ico(images):
    """ICO with PNG-compressed entries (Windows Vista and later)."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b"", b""
    for size, png in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    return head + entries + data


ICNS_TYPES = [  # (OSType, pixel size)
    (b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256), (b"ic09", 512),
    (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
]


def icns(pngs):
    body = b"".join(t + struct.pack(">I", 8 + len(pngs[s])) + pngs[s] for t, s in ICNS_TYPES)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def svg():
    """Vector master (macOS geometry at 1024 px)."""
    g = geometry(1024, True)
    s = 1024
    m, c = g["margin"] * s, g["corner"] * s
    r, w, dot = g["r"] * s, g["w"] * s, g["dot"] * s

    def pt(a):
        return s / 2 + r * math.cos(math.radians(a)), s / 2 - r * math.sin(math.radians(a))

    def arc(sweep):
        (x0, y0), (x1, y1) = pt(START), pt(START - sweep)
        large = 1 if sweep > 180 else 0
        return f"M{x0:.1f} {y0:.1f} A{r:.1f} {r:.1f} 0 {large} 1 {x1:.1f} {y1:.1f}"

    hexc = lambda c: "#%02X%02X%02X" % c  # noqa: E731
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}" width="{s}" height="{s}">
  <!-- MemManager app icon. Generated by scripts/make_icons.py; edit that script, not this file. -->
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{hexc(TOP)}"/>
      <stop offset="1" stop-color="{hexc(BOTTOM)}"/>
    </linearGradient>
  </defs>
  <rect x="{m:.1f}" y="{m:.1f}" width="{s - 2 * m:.1f}" height="{s - 2 * m:.1f}" rx="{c:.1f}" fill="url(#bg)"/>
  <g fill="none" stroke="#FFFFFF" stroke-width="{w:.1f}" stroke-linecap="round">
    <path d="{arc(SWEEP)}" stroke-opacity="0.32"/>
    <path d="{arc(SWEEP * FILL)}"/>
  </g>
  <circle cx="{s / 2:.1f}" cy="{s / 2:.1f}" r="{dot:.1f}" fill="#FFFFFF"/>
</svg>
"""


def write(rel, data):
    path = os.path.join(ROOT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    print(f"{rel}: {len(data):,} bytes")


def main():
    win = [(s, render(s, False)) for s in (16, 20, 24, 32, 40, 48, 64, 256)]
    write("windows/app/memmanager.ico", ico(win))
    mac = {s: render(s, True) for s in sorted({s for _, s in ICNS_TYPES})}
    write("macos/Resources/AppIcon.icns", icns(mac))
    write("assets/icon.svg", svg().encode())
    write("assets/icon-256.png", mac[256])


if __name__ == "__main__":
    main()
