#!/usr/bin/env python3
"""Measures the symbol's gradient from the owner's gradient master.

    python3 scripts/brand/fit_gradient.py

Prints SYMBOL_GRADIENT_ANGLE and SYMBOL_STOPS for scripts/brand/compose.py. Re-run only if the
owner replaces docs/design/concepts/masters/symbol-gradient.png, then paste the output into
compose.py.

The master's color is not a plain left-to-right ramp: it runs cyan -> blue -> violet -> blue ->
cyan along a direction tilted slightly upward, so the crescent's lower left is the most violet and
its upper right the most cyan. SVG can only draw linear or radial gradients, so this finds the
linear one that best reproduces the master: for each candidate angle it projects every inked
pixel onto that direction, splits the projection into STOP_COUNT equal bins, takes each bin's
mean color as a stop, and keeps the angle whose stops leave the smallest color error.

Needs `magick` (ImageMagick 7) and numpy.
"""

import subprocess
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
MASTERS = ROOT / "docs/design/concepts/masters"
STOP_COUNT = 16
# Sampled at a quarter of the master's size: plenty of pixels per stop, and fast.
SAMPLE_SCALE = 0.25


def raw(path: Path, crop: str, colorspace: list[str], depth: str) -> np.ndarray:
    out = subprocess.run(
        ["magick", str(path), "-crop", crop, "+repage", "-resize", f"{SAMPLE_SCALE * 100}%",
         *colorspace, "-depth", "8", f"{depth}:-"],
        check=True, capture_output=True,
    ).stdout
    return np.frombuffer(out, np.uint8)


def main() -> None:
    # The symbol's ink box in the masters (the same in both; trace.py derives it the same way).
    symbol = (ROOT / "assets/brand/symbol-navy.svg").read_text()
    w, h = (float(v) for v in symbol.split('viewBox="0 0 ')[1].split('"')[0].split())
    bbox = subprocess.run(
        ["magick", str(MASTERS / "symbol-navy.png"), "(", "+clone", "-fill", "white",
         "-colorize", "100", ")", "-compose", "difference", "-composite", "-colorspace", "gray",
         "-threshold", "30%", "-format", "%@", "info:"],
        check=True, capture_output=True, text=True,
    ).stdout
    sw, sh = round(w * SAMPLE_SCALE), round(h * SAMPLE_SCALE)
    color = raw(MASTERS / "symbol-gradient.png", bbox, [], "rgb").reshape(sh, sw, 3)
    shape = raw(MASTERS / "symbol-navy.png", bbox, ["-colorspace", "gray"], "gray")
    ink = shape.reshape(sh, sw) < 60
    ys, xs = np.nonzero(ink)
    pixels = color[ys, xs].astype(float)
    x, y = xs / SAMPLE_SCALE, ys / SAMPLE_SCALE  # back in master pixels

    best = None
    for angle in np.arange(-30.0, 30.5, 0.5):
        t = np.radians(angle)
        u = x * np.cos(t) + y * np.sin(t)
        lo, hi = u.min(), u.max()
        bins = np.minimum(((u - lo) / (hi - lo) * STOP_COUNT).astype(int), STOP_COUNT - 1)
        error, stops = 0.0, []
        for b in range(STOP_COUNT):
            sel = bins == b
            mean = pixels[sel].mean(0)
            error += ((pixels[sel] - mean) ** 2).sum()
            stops.append(((b + 0.5) / STOP_COUNT, mean))
        if best is None or error < best[0]:
            best = (error, angle, lo, hi, stops)
    error, angle, lo, hi, stops = best
    t = np.radians(angle)
    rms = np.sqrt(error / pixels.size)
    print(f"# Fitted by scripts/brand/fit_gradient.py: RMS color error {rms:.1f} of 255.")
    print(f"SYMBOL_GRADIENT_LINE = ({lo * np.cos(t):.1f}, {lo * np.sin(t):.1f}, "
          f"{hi * np.cos(t):.1f}, {hi * np.sin(t):.1f})  # x1, y1, x2, y2; {angle:g} degrees")
    print("SYMBOL_STOPS = [")
    for offset, mean in stops:
        print(f'    ({offset:.4f}, "#{"".join(f"{round(v):02X}" for v in mean)}"),')
    print("]")


if __name__ == "__main__":
    main()
