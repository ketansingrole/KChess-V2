#!/usr/bin/env python3
"""Draw the KChess app icon and write every size and format the app needs.

The icon is original artwork made of a few geometric shapes (a rook on a rounded
tile), so it can ship under the project's licence. Run from the repository root:

    python3 scripts/make-icons.py

Needs Pillow (`pip install pillow`); the .icns file also needs macOS' `iconutil`.
Outputs: build/icon.svg, build/icon.png, build/icon.ico, build/icon.icns,
build/icon-{light,dark}.png, and copies in public/ and app/assets/.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageOps

ROOT = Path(__file__).resolve().parent.parent
DESIGN = 1024  # the coordinate space the shapes below are drawn in
SUPERSAMPLE = 3  # draw large, then shrink, for smooth edges

# ── The rook: (kind, coordinates, corner radius) in DESIGN space ─────────────────
RookShape = tuple[str, tuple[float, ...], float]
ROOK: list[RookShape] = [
    ("rrect", (282, 745, 742, 800), 18),  # base plate
    ("rrect", (312, 690, 712, 752), 16),  # foot
    ("poly", (382, 395, 642, 395, 682, 700, 342, 700), 0),  # body, wider at the bottom
    ("rrect", (322, 350, 702, 402), 12),  # collar
    ("rrect", (332, 300, 692, 352), 0),  # band joining the battlements
    ("rrect", (332, 250, 422, 352), 7),  # left merlon
    ("rrect", (467, 250, 557, 352), 7),  # middle merlon
    ("rrect", (602, 250, 692, 352), 7),  # right merlon
]
ROOK_SCALE = 1.12  # how much of the tile the rook fills
ROOK_CENTER = (512, 525)  # centre of the rook in DESIGN space

# Light tile with a dark rook, and the reverse for dark mode.
THEMES = {
    "light": {
        "tile": ("#F7DC90", "#D8A23F"),
        "rook": ("#3B2D20", "#1E1610"),
        "edge": "#B98524",
    },
    "dark": {
        "tile": ("#7B5D30", "#35260F"),
        "rook": ("#FCF2D8", "#E6D09B"),
        "edge": "#1E1408",
    },
}

CORNER = 0.2237  # macOS-style corner radius as a share of the tile


def place(x: float, y: float) -> tuple[float, float]:
    """Move a DESIGN-space point so the rook is centred and scaled within the tile."""
    cx, cy = ROOK_CENTER
    return (
        DESIGN / 2 + (x - cx) * ROOK_SCALE,
        DESIGN / 2 + (y - cy) * ROOK_SCALE,
    )


def rook_mask(size: int) -> Image.Image:
    """The rook silhouette as an 'L' mask, `size` pixels square (size = tile size)."""
    k = size / DESIGN
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    for kind, coords, radius in ROOK:
        if kind == "poly":
            points = [place(coords[i], coords[i + 1]) for i in range(0, len(coords), 2)]
            draw.polygon([(x * k, y * k) for x, y in points], fill=255)
        else:
            x0, y0 = place(coords[0], coords[1])
            x1, y1 = place(coords[2], coords[3])
            draw.rounded_rectangle(
                (x0 * k, y0 * k, x1 * k, y1 * k), radius=radius * ROOK_SCALE * k, fill=255
            )
    return mask


def vertical_gradient(size: int, top: str, bottom: str) -> Image.Image:
    gradient = Image.linear_gradient("L").resize((size, size))
    return ImageOps.colorize(gradient, black=top, white=bottom).convert("RGBA")


def render_tile(theme: str, size: int) -> Image.Image:
    """The icon tile (rounded square, gradient, rook with a soft shadow) at `size` pixels."""
    colors = THEMES[theme]
    big = size * SUPERSAMPLE
    tile_mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(tile_mask).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=int(big * CORNER), fill=255
    )
    tile = vertical_gradient(big, *colors["tile"])
    tile.putalpha(tile_mask)

    rook = rook_mask(big)
    shadow = rook.filter(ImageFilter.GaussianBlur(big * 0.012)).point(lambda v: int(v * 0.38))
    shadow_layer = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    shadow_layer.paste(
        Image.new("RGBA", (big, big), (20, 10, 0, 255)), (0, int(big * 0.014)), mask=shadow
    )
    tile = Image.alpha_composite(tile, shadow_layer)

    fill = vertical_gradient(big, *colors["rook"])
    tile.paste(fill, (0, 0), mask=rook)

    # A thin darker edge keeps the tile readable on light backgrounds.
    edge = Image.new("L", (big, big), 0)
    ImageDraw.Draw(edge).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=int(big * CORNER), outline=255, width=max(2, big // 256)
    )
    edge_layer = Image.new("RGBA", (big, big), colors["edge"])
    edge_layer.putalpha(edge.point(lambda v: int(v * 0.55)))
    tile = Image.alpha_composite(tile, edge_layer)
    return tile.resize((size, size), Image.LANCZOS)


def framed(theme: str, size: int, inset: float) -> Image.Image:
    """The tile on a transparent square; `inset` leaves the margin macOS icons have."""
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    inner = round(size * (1 - 2 * inset))
    tile = render_tile(theme, inner)
    canvas.paste(tile, (round(size * inset), round(size * inset)), tile)
    return canvas


def svg() -> str:
    """The light icon as an SVG, from the same shape list."""
    light = THEMES["light"]
    shapes = []
    for kind, c, radius in ROOK:
        if kind == "poly":
            pts = " ".join(f"{x:.1f},{y:.1f}" for x, y in (place(c[i], c[i + 1]) for i in range(0, len(c), 2)))
            shapes.append(f'    <polygon points="{pts}"/>')
        else:
            (x0, y0), (x1, y1) = place(c[0], c[1]), place(c[2], c[3])
            shapes.append(
                f'    <rect x="{x0:.1f}" y="{y0:.1f}" width="{x1 - x0:.1f}" height="{y1 - y0:.1f}" '
                f'rx="{radius * ROOK_SCALE:.1f}"/>'
            )
    body = "\n".join(shapes)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {DESIGN} {DESIGN}" width="512" height="512">
  <title>KChess</title>
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{light['tile'][0]}"/>
      <stop offset="1" stop-color="{light['tile'][1]}"/>
    </linearGradient>
    <linearGradient id="rook" gradientUnits="userSpaceOnUse" x1="0" y1="{place(0, 250)[1]:.0f}" x2="0" y2="{place(0, 800)[1]:.0f}">
      <stop offset="0" stop-color="{light['rook'][0]}"/>
      <stop offset="1" stop-color="{light['rook'][1]}"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="14" stdDeviation="12" flood-color="#140a00" flood-opacity="0.38"/>
    </filter>
  </defs>
  <rect width="{DESIGN}" height="{DESIGN}" rx="{DESIGN * CORNER:.0f}" fill="url(#tile)"/>
  <g fill="url(#rook)" filter="url(#shadow)">
{body}
  </g>
</svg>
"""


def write_icns(master: Image.Image, target: Path) -> None:
    if sys.platform != "darwin" or not shutil.which("iconutil"):
        print("skipped build/icon.icns (needs macOS iconutil)")
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "icon.iconset"
        iconset.mkdir()
        for base in (16, 32, 128, 256, 512):
            master.resize((base, base), Image.LANCZOS).save(iconset / f"icon_{base}x{base}.png")
            master.resize((base * 2, base * 2), Image.LANCZOS).save(
                iconset / f"icon_{base}x{base}@2x.png"
            )
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(target)], check=True)


def main() -> None:
    build, public, assets = ROOT / "build", ROOT / "public", ROOT / "app" / "assets"
    for folder in (build, public, assets):
        folder.mkdir(parents=True, exist_ok=True)

    (build / "icon.svg").write_text(svg())

    full = framed("light", 512, 0.0)  # edge to edge: window icon, splash, Linux
    inset_light = framed("light", 512, 0.07)  # macOS Dock look, with the usual margin
    inset_dark = framed("dark", 512, 0.07)
    for folder in (build, public):
        full.save(folder / "icon.png")
        inset_light.save(folder / "icon-light.png")
        inset_dark.save(folder / "icon-dark.png")
    full.save(assets / "icon.png")

    framed("light", 256, 0.0).save(
        build / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]
    )
    write_icns(framed("light", 1024, 0.07), build / "icon.icns")
    print("icons written")


if __name__ == "__main__":
    main()
