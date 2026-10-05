#!/usr/bin/env python3
"""Composes every Tachyon logo and icon SVG from the two traced masters.

    python3 scripts/brand/compose.py

Reads (written by trace.py, never edited by hand):
    assets/brand/symbol-navy.svg, assets/brand/wordmark-navy.svg

Writes into assets/brand/ (all committed):
    symbol-gradient.svg, symbol-white.svg       the symbol in its other two colorways
    wordmark-white.svg                          the wordmark for dark surfaces
    lockup-horizontal.svg, lockup-horizontal-white.svg
    lockup-stacked.svg, lockup-stacked-white.svg
    about-lockup.svg, about-lockup-white.svg    the stacked lockup sized for the About window
    app-icon.svg                                the app icon, 96 px and up (256 px grid)
    app-icon-16.svg ... app-icon-64.svg         the app icon drawn on each smaller pixel grid
    file-markdown-<n>.svg, file-text-<n>.svg    file-type icons for 16, 24, 32, 48 and 256 px

Standard library only; no external tools. Every number below is either a brand token from the
Claude Design system ("Tachyon Design System", tokens/colors.css; mirrored in
docs/design/system/) or measured from the owner's concept sheets, as noted beside it. Change a
value only together with the source it cites.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BRAND = ROOT / "assets/brand"

# Brand tokens (tokens/colors.css).
CYAN = "#00D1FF"
BLUE = "#2563FF"
VIOLET = "#7C3AED"
NAVY = "#0B1220"
WHITE = "#FFFFFF"

# The symbol's gradient, measured from docs/design/concepts/masters/symbol-gradient.png by
# scripts/brand/fit_gradient.py (paste its output here if the master ever changes). The colors
# are the master's own, not snapped to the brand tokens: this is the owner's artwork, and the
# measured violet (#622FFC) is bluer than the Tachyon Violet token (#7C3AED) that UI uses.
# Fitted by scripts/brand/fit_gradient.py: RMS color error 15.5 of 255.
SYMBOL_GRADIENT_LINE = (-26.2, 4.6, 1059.3, -186.8)  # x1, y1, x2, y2; -10 degrees
SYMBOL_STOPS = [
    (0.0312, "#0FC9FB"),
    (0.0938, "#14B1FD"),
    (0.1562, "#209AFD"),
    (0.2188, "#366FFC"),
    (0.2812, "#4A4CFC"),
    (0.3438, "#622FFC"),
    (0.4062, "#5F2AFC"),
    (0.4688, "#443CFC"),
    (0.5312, "#235CFD"),
    (0.5938, "#0D80FD"),
    (0.6562, "#04AFFD"),
    (0.7188, "#02CFFD"),
    (0.7812, "#04D6FD"),
    (0.8438, "#06D2FD"),
    (0.9062, "#0BCFFC"),
    (0.9688, "#13CFFB"),
]

# Lockup proportions, measured from concept sheet 1 (docs/design/concepts/, "Primary Logo
# (Stacked)" and "Horizontal Logo"), in multiples of the wordmark's height.
# Symbol height. The sheet's symbol is 0.66 of the wordmark's width; with the master symbol's
# proportions (1158 x 501) that is 1.22 of the wordmark's height.
HORIZONTAL_SYMBOL_HEIGHT = 1.22
HORIZONTAL_GAP = 0.20  # symbol's right tip to the wordmark
STACKED_SYMBOL_WIDTH = 1.01  # symbol width as a multiple of the wordmark's width
STACKED_GAP = 0.22  # symbol bottom to wordmark top
# The About window shows the stacked lockup this many px wide (crates/tachyon-editor/src/about.rs,
# LOCKUP_WIDTH; keep the two equal).
ABOUT_LOCKUP_WIDTH = 220

# App icons (owner decision, 2026-10-02): the concept's dark-mode tile (concept sheet 2), navy,
# with a colored border. Each size an icon is shipped at gets art drawn on its own pixel grid,
# because one drawing scaled across 16-256 px is either too thin small or too crude large.
# `cargo xtask icons` renders each requested size from the smallest art at least that large.
#
# Per size: (pixel grid, cut, thicken px, border px, border paint, glow)
# - cut "full": the whole symbol. "simple": concept sheet 2's simplified small mark, which drops
#   the short free-standing speed line (the symbol's narrowest part) and crops the spike's thin
#   outer tips to SIMPLE_CROP so the core can be drawn larger.
# - thicken: every part of the mark grows by this many pixels, so hairlines survive.
# - border: "gradient" is the brand gradient, cyan top left to violet bottom right; a gradient
#   one pixel wide reads as mud below 40 px, so the small sizes use one solid Tachyon Blue pixel.
# - glow: the soft blurred copy of the mark behind it, as on the concept's dark tile; only where
#   there are enough pixels for it to read as light rather than smear.
ICONS = {
    "app-icon.svg": (256, "full", 0, 6, "gradient", True),
    "app-icon-64.svg": (64, "full", 0.6, 1, "gradient", False),
    "app-icon-48.svg": (48, "full", 0.55, 1, "gradient", False),
    "app-icon-40.svg": (40, "full", 0.5, 1, "gradient", False),
    "app-icon-32.svg": (32, "simple", 0.71, 1, "blue", False),
    "app-icon-24.svg": (24, "simple", 0.61, 1, "blue", False),
    "app-icon-20.svg": (20, "simple", 0.56, 1, "blue", False),
    "app-icon-16.svg": (16, "simple", 0.5, 1, "blue", False),
}
ICON_MARGIN = 12 / 256  # transparent margin around the tile, as a share of the grid
ICON_RADIUS = 0.22  # tile corner radius as a share of the tile (concept sheet 2)
# Mark width as a share of the tile's inside. Concept sheet 2's spike tips run almost to the tile
# edge; the simplified cut is already cropped, so it keeps a little air.
ICON_MARK_WIDTH = {"full": 0.98, "simple": 0.90}
ICON_GLOW = 14 / 256  # glow blur radius as a share of the grid
ICON_GLOW_OPACITY = 0.45
# The concept's dark tile is faintly lighter at its center: a radial gradient between two of the
# design system's dark surface tokens (navy-750 at the center, navy-900 at the edge).
TILE_CENTER = "#141D33"
TILE_EDGE = NAVY
SIMPLE_CROP = (100, 980)  # x range of the symbol kept by the "simple" cut, in master pixels
SMALL_MARGIN = 0  # the 16-64 px tiles use the whole grid: every pixel counts at these sizes

# File-type icons: what Windows shows on .md/.markdown and .txt/.text/.log files Tachyon opens
# (packaging/msix/AppxManifest.xml, <uap:Logo> per FileTypeAssociation). A document page, the
# Windows convention for files, so a file never looks like the app itself: white with a folded
# top-right corner, carrying the symbol as the app's badge. Markdown and plain text differ in two
# ways, so neither relies on color alone: the Markdown page has a heading line above its text
# lines and the gradient symbol; the plain-text page has even lines and the navy symbol.
#
# Per size: (pixel grid, cut, thicken px, draw text lines)
FILE_ICONS = {
    16: ("simple", 0.5, False),
    24: ("simple", 0.6, False),
    32: ("simple", 0.7, True),
    48: ("full", 0.55, True),
    256: ("full", 0, True),
}
PAGE_X = (0.16, 0.84)  # page left and right edges, as shares of the grid
PAGE_Y = (0.04, 0.96)  # page top and bottom
PAGE_FOLD = 0.30  # folded corner size, as a share of the page's width
PAGE_RADIUS = 0.04  # page corner radius, as a share of the grid
PAGE_FILL = WHITE
PAGE_EDGE = "#94A3B8"  # neutral-400: the page edge holds 3:1 against white and #F3F3F3
PAGE_FOLD_FILL = "#E2E8F0"  # neutral-200
PAGE_LINE = "#CBD5E1"  # neutral-300: text lines, decorative
PAGE_HEADING = "#64748B"  # neutral-500: the Markdown page's heading line
FILE_MARK_WIDTH = 0.86  # symbol width as a share of the page's inside
FILE_MARK_CENTER = 0.72  # symbol center height, as a share of the page's height


def read_master(name: str) -> tuple[float, float, str]:
    svg = (BRAND / name).read_text()
    w, h = map(float, re.search(r'viewBox="0 0 ([\d.]+) ([\d.]+)"', svg).groups())
    return w, h, re.search(r' d="([^"]+)"', svg).group(1)


SYM_W, SYM_H, SYM_D = read_master("symbol-navy.svg")
WORD_W, WORD_H, WORD_D = read_master("wordmark-navy.svg")


def fmt(v: float) -> str:
    return ("%.3f" % v).rstrip("0").rstrip(".")


def header(w: float, h: float, note: str, width: float | None = None) -> str:
    """The opening tag: a `w` x `h` viewBox, declared `width` px wide (default `w`)."""
    dw = width or w
    dh = h * dw / w
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(w)} {fmt(h)}" '
        f'width="{fmt(dw)}" height="{fmt(dh)}">\n'
        f"<!-- {note} Generated by scripts/brand/compose.py; do not edit by hand. -->\n"
    )


def symbol_gradient(gid: str) -> str:
    stops = "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in SYMBOL_STOPS)
    x1, y1, x2, y2 = SYMBOL_GRADIENT_LINE
    return (f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="{fmt(x1)}" '
            f'y1="{fmt(y1)}" x2="{fmt(x2)}" y2="{fmt(y2)}">{stops}</linearGradient>')


def placed(d: str, fill: str, x: float, y: float, scale: float, extra: str = "") -> str:
    return (f'<path transform="translate({fmt(x)} {fmt(y)}) scale({fmt(scale)})" '
            f'fill="{fill}"{extra} d="{d}"/>')


def write(name: str, text: str) -> None:
    (BRAND / name).write_text(text + "</svg>\n")
    print(f"assets/brand/{name}")


def symbols() -> None:
    write("symbol-gradient.svg", header(SYM_W, SYM_H, "The symbol, brand gradient.")
          + f"<defs>{symbol_gradient('g')}</defs>\n" + f'<path fill="url(#g)" d="{SYM_D}"/>\n')
    write("symbol-white.svg", header(SYM_W, SYM_H, "The symbol, white, for dark surfaces.")
          + f'<path fill="{WHITE}" d="{SYM_D}"/>\n')
    write("wordmark-white.svg", header(WORD_W, WORD_H, "The wordmark, white, for dark surfaces.")
          + f'<path fill="{WHITE}" d="{WORD_D}"/>\n')


def lockups() -> None:
    for suffix, ink in (("", NAVY), ("-white", WHITE)):
        # Horizontal: symbol left, wordmark right, centered on each other vertically.
        sh = WORD_H * HORIZONTAL_SYMBOL_HEIGHT
        s = sh / SYM_H
        sw = SYM_W * s
        gap = WORD_H * HORIZONTAL_GAP
        w, h = sw + gap + WORD_W, max(sh, WORD_H)
        write(f"lockup-horizontal{suffix}.svg",
              header(w, h, "Horizontal lockup: gradient symbol and wordmark.")
              + f"<defs>{symbol_gradient('g')}</defs>\n"
              + placed(SYM_D, "url(#g)", 0, (h - sh) / 2, s) + "\n"
              + placed(WORD_D, ink, sw + gap, (h - WORD_H) / 2, 1) + "\n")
        # Stacked: symbol above wordmark, centered on each other horizontally.
        sw = WORD_W * STACKED_SYMBOL_WIDTH
        s = sw / SYM_W
        sh = SYM_H * s
        gap = WORD_H * STACKED_GAP
        w, h = max(sw, WORD_W), sh + gap + WORD_H
        artwork = (f"<defs>{symbol_gradient('g')}</defs>\n"
                   + placed(SYM_D, "url(#g)", (w - sw) / 2, 0, s) + "\n"
                   + placed(WORD_D, ink, (w - WORD_W) / 2, sh + gap, 1) + "\n")
        write(f"lockup-stacked{suffix}.svg",
              header(w, h, "Stacked lockup: gradient symbol above the wordmark.") + artwork)
        # The same artwork declared at twice the About window's display width. GPUI rasterizes
        # an SVG image once, at its declared size, then scales the bitmap: declared at the
        # master's 1090 px it would be shrunk about 4x (aliased edges); at 2x it is exact at
        # 200 % scaling and a clean 2:1 reduction at 100 %.
        write(f"about-lockup{suffix}.svg",
              header(w, h, "Stacked lockup sized for the About window.",
                     width=2 * ABOUT_LOCKUP_WIDTH) + artwork)


def simple_symbol() -> str:
    """The symbol without its narrowest part (the short free-standing speed line): the
    "simple" cut's path data, still to be cropped to SIMPLE_CROP by whoever draws it."""
    parts = re.findall(r"M[^M]*", SYM_D)

    def part_width(part: str) -> float:
        xs = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", part)[0::2]]
        return max(xs) - min(xs)

    shortest = min(parts, key=part_width)
    return "".join(p for p in parts if p is not shortest)


def icons() -> None:
    simple_d = simple_symbol()
    for name, (grid, cut, thicken, border, paint, glow) in ICONS.items():
        margin = grid * ICON_MARGIN if grid > 64 else SMALL_MARGIN
        tile = grid - 2 * margin
        radius = tile * ICON_RADIUS
        # The border is centered on its own rectangle, so that rectangle is inset by half of it.
        edge = margin + border / 2
        inside = tile - 2 * border
        if cut == "full":
            d, x0, x1 = SYM_D, 0.0, SYM_W
        else:
            d, (x0, x1) = simple_d, SIMPLE_CROP
        mark_w = inside * ICON_MARK_WIDTH[cut]
        s = mark_w / (x1 - x0)
        x, y = (grid - mark_w) / 2 - x0 * s, (grid - SYM_H * s) / 2
        stroke = (f' stroke="url(#g)" stroke-width="{fmt(thicken / s)}" stroke-linejoin="round"'
                  if thicken else "")
        border_paint = "url(#b)" if paint == "gradient" else BLUE
        defs = (
            symbol_gradient("g")
            + f'<linearGradient id="b" x1="0" y1="0" x2="1" y2="1"><stop offset="0" '
              f'stop-color="{CYAN}"/><stop offset=".5" stop-color="{BLUE}"/><stop offset="1" '
              f'stop-color="{VIOLET}"/></linearGradient>'
            + f'<radialGradient id="t" cx=".5" cy=".42" r=".7"><stop offset="0" '
              f'stop-color="{TILE_CENTER}"/><stop offset="1" stop-color="{TILE_EDGE}"/>'
              f"</radialGradient>"
            # The mark is clipped to the tile's inside so a cropped spike never crosses the border.
            + f'<clipPath id="c"><rect x="{fmt(margin + border)}" y="{fmt(margin + border)}" '
              f'width="{fmt(inside)}" height="{fmt(inside)}" rx="{fmt(radius - border)}"/>'
              f"</clipPath>"
        )
        body = (
            f'<rect x="{fmt(edge)}" y="{fmt(edge)}" width="{fmt(grid - 2 * edge)}" '
            f'height="{fmt(grid - 2 * edge)}" rx="{fmt(radius - border / 2)}" fill="url(#t)" '
            f'stroke="{border_paint}" stroke-width="{fmt(border)}"/>\n'
        )
        if glow:
            defs += (f'<filter id="glow" x="-20%" y="-50%" width="140%" height="200%">'
                     f'<feGaussianBlur stdDeviation="{fmt(grid * ICON_GLOW)}"/></filter>')
            body += (f'<g opacity="{ICON_GLOW_OPACITY}" filter="url(#glow)">'
                     + placed(d, "url(#g)", x, y, s) + "</g>\n")
        body += '<g clip-path="url(#c)">' + placed(d, "url(#g)", x, y, s, stroke) + "</g>\n"
        note = (f"App icon art for {grid} px and the sizes just below it." if grid <= 64
                else "App icon art for 96 px and up, and the Linux launcher.")
        write(name, header(grid, grid, note) + f"<defs>{defs}</defs>\n" + body)


def file_icons() -> None:
    simple_d = simple_symbol()
    for kind, mark_fill, heading in (("markdown", "url(#g)", True), ("text", NAVY, False)):
        for grid, (cut, thicken, lines) in FILE_ICONS.items():
            # Snapped to half pixels so the 1 px edge lands crisply on the pixel grid.
            snap = (lambda v: round(v * 2) / 2) if grid < 64 else (lambda v: v)
            x0, x1 = snap(grid * PAGE_X[0]) + 0.5, snap(grid * PAGE_X[1]) - 0.5
            y0, y1 = snap(grid * PAGE_Y[0]) + 0.5, snap(grid * PAGE_Y[1]) - 0.5
            edge = max(1.0, grid / 128)
            w, h = x1 - x0, y1 - y0
            fold = snap(w * PAGE_FOLD)
            r = grid * PAGE_RADIUS
            page = (f"M{fmt(x0 + r)} {fmt(y0)}H{fmt(x1 - fold)}L{fmt(x1)} {fmt(y0 + fold)}"
                    f"V{fmt(y1 - r)}Q{fmt(x1)} {fmt(y1)} {fmt(x1 - r)} {fmt(y1)}H{fmt(x0 + r)}"
                    f"Q{fmt(x0)} {fmt(y1)} {fmt(x0)} {fmt(y1 - r)}V{fmt(y0 + r)}"
                    f"Q{fmt(x0)} {fmt(y0)} {fmt(x0 + r)} {fmt(y0)}Z")
            flap = (f"M{fmt(x1 - fold)} {fmt(y0)}V{fmt(y0 + fold - r)}"
                    f"Q{fmt(x1 - fold)} {fmt(y0 + fold)} {fmt(x1 - fold + r)} {fmt(y0 + fold)}"
                    f"H{fmt(x1)}Z")
            body = (f'<path d="{page}" fill="{PAGE_FILL}" stroke="{PAGE_EDGE}" '
                    f'stroke-width="{fmt(edge)}" stroke-linejoin="round"/>\n'
                    f'<path d="{flap}" fill="{PAGE_FOLD_FILL}" stroke="{PAGE_EDGE}" '
                    f'stroke-width="{fmt(edge)}" stroke-linejoin="round"/>\n')
            inside = w - 2 * edge
            if lines:
                # Text lines between the top edge and the symbol, left-aligned like text.
                lx = x0 + inside * 0.14 + edge
                lw = inside * 0.72
                lh = max(1.0, round(grid * 0.035))
                gap = max(2.0, round(grid * 0.075))
                ly = snap(y0 + fold + gap * 0.6)
                rows = [(PAGE_HEADING, 0.55, lh * 1.6)] if heading else []
                rows += [(PAGE_LINE, 1.0, lh), (PAGE_LINE, 0.85, lh), (PAGE_LINE, 0.95, lh)]
                limit = y0 + h * FILE_MARK_CENTER - h * 0.17
                for color, share, height in rows:
                    if ly + height > limit:
                        break
                    width = lw * share if color == PAGE_HEADING else (lw - fold * 0.2) * share
                    body += (f'<rect x="{fmt(lx)}" y="{fmt(ly)}" width="{fmt(width)}" '
                             f'height="{fmt(height)}" rx="{fmt(height / 2)}" fill="{color}"/>\n')
                    ly += height + gap * 0.55
            if cut == "full":
                d, c0, c1 = SYM_D, 0.0, SYM_W
            else:
                d, (c0, c1) = simple_d, SIMPLE_CROP
            mark_w = inside * FILE_MARK_WIDTH
            sc = mark_w / (c1 - c0)
            mx = (x0 + x1) / 2 - mark_w / 2 - c0 * sc
            my = y0 + h * FILE_MARK_CENTER - SYM_H * sc / 2
            stroke = (f' stroke="{mark_fill}" stroke-width="{fmt(thicken / sc)}" '
                      f'stroke-linejoin="round"' if thicken else "")
            clip = (f'<clipPath id="c"><rect x="{fmt(x0 + edge)}" y="{fmt(y0)}" '
                    f'width="{fmt(inside)}" height="{fmt(h)}"/></clipPath>')
            body += '<g clip-path="url(#c)">' + placed(d, mark_fill, mx, my, sc, stroke) + "</g>\n"
            name = f"file-{kind}-{grid}.svg"
            note = f"{'Markdown' if heading else 'Plain-text'} file icon drawn for {grid} px."
            write(name, header(grid, grid, note) + f"<defs>{symbol_gradient('g')}{clip}</defs>\n"
                  + body)


if __name__ == "__main__":
    symbols()
    lockups()
    icons()
    file_icons()
