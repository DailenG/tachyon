#!/usr/bin/env python3
"""Traces the owner's master logo rasters into the canonical vector geometry.

    python3 scripts/brand/trace.py

Inputs (owner-supplied masters, never edited):
    docs/design/concepts/masters/symbol-navy.png    the symbol, flat navy on white
    docs/design/concepts/masters/wordmark-navy.png  the italic wordmark, flat navy on white

Outputs (committed; every other logo file is composed from these by compose.py):
    assets/brand/symbol-navy.svg
    assets/brand/wordmark-navy.svg

Needs `magick` (ImageMagick 7), `potrace` and `rsvg-convert` on PATH. Standard library only.

Method, and why each step exists:
1. Upscale the anti-aliased master 4x and blur it slightly, then threshold at 50 %. The edge
   lands with sub-pixel accuracy and the AI-render pixel noise is smoothed out before tracing,
   so potrace emits long smooth curves instead of a faceted outline.
2. potrace the bitmap. Corner threshold (`-a`) and curve optimization (`-O`) were chosen by
   measuring the mismatch below for several settings; see BRAND_SETTINGS.
3. Rewrite potrace's relative, transformed, 10x-quantized path into absolute coordinates in
   the master's own pixel space, shifted so the artwork's bounding box starts at (0, 0).
4. Render the result back at the master's size and count the pixels where the two disagree.
   The script fails, writing nothing, if that mismatch exceeds MAX_MISMATCH of the inked area:
   the vector must stay the owner's drawing, not an approximation of it.

Do not hand-edit the output paths. If the geometry needs to change, the owner supplies a new
master and this script is re-run.
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MASTERS = ROOT / "docs/design/concepts/masters"
BRAND = ROOT / "assets/brand"
NAVY = "#0B1220"
UPSCALE = 4
# Mismatch allowed between the traced vector and its master, as a share of the inked pixels.
# The committed traces measure about 0.35 % (symbol) and 0.39 % (wordmark): under a tenth of a
# pixel of average edge offset.
MAX_MISMATCH = 0.005

# name: (master file, output file, blur sigma at 4x, potrace -a, potrace -O)
BRAND_SETTINGS = {
    "symbol": ("symbol-navy.png", "symbol-navy.svg", 2.5, 1.2, 1.0),
    "wordmark": ("wordmark-navy.png", "wordmark-navy.svg", 1.5, 1.2, 0.8),
}


def run(*args: str) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def ink_bbox(master: Path) -> tuple[int, int, int, int]:
    """Bounding box (x, y, w, h) of everything that is not the white background."""
    geometry = run(
        "magick", str(master),
        "(", "+clone", "-fill", "white", "-colorize", "100", ")",
        "-compose", "difference", "-composite", "-colorspace", "gray",
        "-threshold", "30%", "-format", "%@", "info:",
    )
    w, h, x, y = map(int, re.match(r"(\d+)x(\d+)\+(\d+)\+(\d+)", geometry).groups())
    return x, y, w, h


def potrace_to_absolute(svg: str, ox: float, oy: float) -> str:
    """potrace's single-path SVG -> absolute `d` in master pixels, origin at (ox, oy)."""
    height = float(re.search(r"translate\(0\.0+,([\d.]+)\)", svg).group(1))

    def to_master(x: float, y: float) -> tuple[float, float]:
        # potrace writes 10x-quantized units under translate(0,H) scale(0.1,-0.1).
        return (x * 0.1 / UPSCALE - ox, (height - y * 0.1) / UPSCALE - oy)

    def num(v: float) -> str:
        text = ("%.2f" % v).rstrip("0").rstrip(".")
        return "0" if text == "-0" else text

    out = []
    for d in re.findall(r'<path d="([^"]+)"', svg):
        tokens = re.findall(r"[MmCcLlZz]|-?\d+(?:\.\d+)?", d)
        i, cx, cy, sx, sy, cmd = 0, 0.0, 0.0, 0.0, 0.0, ""
        while i < len(tokens):
            if tokens[i].isalpha():
                cmd = tokens[i]
                i += 1
                if cmd in "Zz":
                    out.append("Z")
                    cx, cy = sx, sy
                continue
            n = {"M": 2, "L": 2, "C": 6}[cmd.upper()]
            values = [float(v) for v in tokens[i:i + n]]
            i += n
            points = []
            for k in range(0, n, 2):
                if cmd.islower():
                    points.append((cx + values[k], cy + values[k + 1]))
                else:
                    points.append((values[k], values[k + 1]))
            cx, cy = points[-1]
            letter = cmd.upper()
            if letter == "M":
                sx, sy = cx, cy
                # Further coordinate pairs after a moveto are implicit linetos.
                cmd = "l" if cmd.islower() else "L"
            out.append(letter + " ".join(
                "%s %s" % tuple(num(c) for c in to_master(*p)) for p in points
            ))
    return "".join(out)


def mismatch(master: Path, svg_path: Path, size: int, work: Path) -> float:
    rendered = work / "rendered.png"
    run("rsvg-convert", "-w", str(size), "-h", str(size), "-b", "white", str(svg_path),
        "-o", str(rendered))
    a = work / "a.png"
    b = work / "b.png"
    run("magick", str(master), "-colorspace", "gray", "-threshold", "50%", str(a))
    run("magick", str(rendered), "-colorspace", "gray", "-threshold", "50%", str(b))
    area = float(run("magick", str(a), "-negate", "-format", "%[fx:mean*w*h]", "info:"))
    xor = float(run("magick", str(a), str(b), "-compose", "difference", "-composite",
                    "-format", "%[fx:mean*w*h]", "info:"))
    return xor / area


def trace(name: str, work: Path) -> tuple[str, str, float]:
    master_name, out_name, blur, alpha, opt = BRAND_SETTINGS[name]
    master = MASTERS / master_name
    size = int(run("magick", "identify", "-format", "%w", str(master)))
    x, y, w, h = ink_bbox(master)
    pbm = work / f"{name}.pbm"
    run("magick", str(master), "-colorspace", "gray", "-resize", f"{UPSCALE * 100}%",
        "-blur", f"0x{blur}", "-threshold", "50%", str(pbm))
    raw = work / f"{name}.raw.svg"
    run("potrace", str(pbm), "-s", "-o", str(raw), "-t", "40", "-a", str(alpha),
        "-O", str(opt), "--flat")
    d = potrace_to_absolute(raw.read_text(), x, y)
    # Score in the master's full frame, so the comparison is pixel for pixel.
    check = work / f"{name}.check.svg"
    check.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-x} {-y} {size} {size}">'
        f'<path fill="{NAVY}" d="{d}"/></svg>'
    )
    ratio = mismatch(master, check, size, work)
    if ratio > MAX_MISMATCH:
        sys.exit(f"{name}: traced vector differs from its master by {ratio:.3%} "
                 f"(limit {MAX_MISMATCH:.1%}); nothing written")
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" '
        f'height="{h}">\n'
        f"<!-- Traced from docs/design/concepts/masters/{master_name} by "
        f"scripts/brand/trace.py ({ratio:.2%} pixel mismatch). Do not edit by hand. -->\n"
        f'<path fill="{NAVY}" d="{d}"/>\n</svg>\n'
    )
    return out_name, svg, ratio


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="tachyon-trace-") as tmp:
        results = [trace(name, Path(tmp)) for name in BRAND_SETTINGS]
    # Written only after every trace passed its check.
    for out_name, svg, ratio in results:
        (BRAND / out_name).write_text(svg)
        print(f"assets/brand/{out_name}: {ratio:.3%} mismatch against its master")


if __name__ == "__main__":
    main()
