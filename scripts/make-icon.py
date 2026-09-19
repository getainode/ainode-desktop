#!/usr/bin/env python3
"""Compose the AINode desktop app icon (1024x1024 PNG with transparency).

The mark is the real AINode logo: the green hexagonal node lattice that ships
in the product web UI (ainode/web/static/img/logo.png in getainode/ainode) and
on ainode.dev. This script places it on a macOS-style rounded tile so the icon
sits correctly next to other Mac apps (Apple's template: an 824 px rounded
square centred on a 1024 px canvas with a soft shadow).

Usage:
    python3 scripts/make-icon.py /path/to/logo.png icon.png
    npx --yes @tauri-apps/cli@latest icon icon.png -o src-tauri/icons/

Needs Pillow (pip install pillow).
"""

import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

CANVAS = 1024
TILE = 824                      # Apple's macOS icon grid: 824 px tile, 100 px margins
RADIUS = 185                    # ~22.4 % of the tile side, the macOS corner radius
SS = 4                          # supersampling factor for anti-aliased shapes

GROUND_TOP = (32, 36, 30)       # dark charcoal with a faint green cast
GROUND_BOTTOM = (11, 13, 10)
GLOW = (118, 185, 0)            # brand green, #76b900
MARK_TOP = (143, 212, 0)        # #8fd400, brand light green
MARK_BOTTOM = (118, 185, 0)     # #76b900

MARK_HEIGHT = 584               # ~71 % of the tile: reads at 128 px, still has air
MARK_DILATE = 25                # MaxFilter size on the source alpha: thickens the
                                # lattice lines so they survive the 32 px sizes


def vertical_gradient(size, top, bottom):
    w, h = size
    img = Image.new("RGB", size)
    px = img.load()
    for y in range(h):
        t = y / max(h - 1, 1)
        c = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
        for x in range(w):
            px[x, y] = c
    return img


def rounded_mask(size, radius, ss=SS):
    big = Image.new("L", (size * ss, size * ss), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, size * ss - 1, size * ss - 1), radius=radius * ss, fill=255
    )
    return big.resize((size, size), Image.LANCZOS)


def radial_glow(size, colour, alpha_max, centre, radius):
    """Soft radial glow layer, RGBA."""
    w, h = size
    alpha = Image.new("L", size, 0)
    px = alpha.load()
    cx, cy = centre
    for y in range(h):
        for x in range(w):
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 / radius
            if d < 1.0:
                px[x, y] = round(alpha_max * 255 * (1 - d) ** 2)
    layer = Image.new("RGBA", size, colour + (0,))
    layer.putalpha(alpha)
    return layer


def build_tile():
    mask = rounded_mask(TILE, RADIUS)
    ground = vertical_gradient((TILE, TILE), GROUND_TOP, GROUND_BOTTOM).convert("RGBA")

    glow = radial_glow((TILE, TILE), GLOW, 0.22, (TILE // 2, int(TILE * 0.46)), int(TILE * 0.62))
    ground.alpha_composite(glow)

    # Subtle inner edge light so the tile has depth on dark desktops.
    edge = Image.new("RGBA", (TILE * SS, TILE * SS), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        (SS // 2, SS // 2, TILE * SS - SS, TILE * SS - SS),
        radius=RADIUS * SS, outline=(255, 255, 255, 28), width=2 * SS,
    )
    edge = edge.resize((TILE, TILE), Image.LANCZOS)
    ground.alpha_composite(edge)

    ground.putalpha(ImageChops.multiply(ground.getchannel("A"), mask))
    return ground


def build_mark(logo_path):
    logo = Image.open(logo_path).convert("RGBA")
    alpha = logo.getchannel("A").filter(ImageFilter.MaxFilter(MARK_DILATE))
    scale = MARK_HEIGHT / logo.height
    size = (round(logo.width * scale), MARK_HEIGHT)
    alpha = alpha.resize(size, Image.LANCZOS)
    fill = vertical_gradient(size, MARK_TOP, MARK_BOTTOM).convert("RGBA")
    fill.putalpha(alpha)
    return fill


def main(logo_path, out_path):
    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    off = (CANVAS - TILE) // 2

    # Drop shadow under the tile, matching Apple's icon template feel.
    shadow = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    sm = rounded_mask(TILE, RADIUS)
    shadow_layer = Image.new("RGBA", (TILE, TILE), (0, 0, 0, 110))
    shadow_layer.putalpha(ImageChops.multiply(shadow_layer.getchannel("A"), sm))
    shadow.paste(shadow_layer, (off, off + 12))
    shadow = shadow.filter(ImageFilter.GaussianBlur(18))
    canvas.alpha_composite(shadow)

    tile = build_tile()
    canvas.alpha_composite(tile, (off, off))

    mark = build_mark(logo_path)
    mx = off + (TILE - mark.width) // 2
    my = off + (TILE - mark.height) // 2
    canvas.alpha_composite(mark, (mx, my))

    canvas.save(out_path, "PNG", optimize=True)
    print(f"wrote {out_path} ({canvas.width}x{canvas.height}, mark {mark.width}x{mark.height})")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: make-icon.py <logo.png> <icon.png>")
    main(sys.argv[1], sys.argv[2])
