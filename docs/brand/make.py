"""Builds Silt's mark and wordmark: SVG, PNGs and the .ico, all from one 8x8 dither grid.

    uv run --with pillow --with fonttools docs/brand/make.py

The mark is sediment settling: an 8x8 grid of grains, sparse at the top, solid at the bottom. Small sizes (16, 24, 32) are drawn cell by cell on whole pixels; larger ones are drawn
at 1024 px and scaled down.
"""

from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from PIL import Image, ImageDraw

HERE = Path(__file__).parent
ROOT = HERE.parent.parent
FONT = ROOT / "crates" / "silt-app" / "assets" / "fonts" / "Archivo.ttf"

DEEP = "#1D3138"  # Deep water: the tile
FLOUR = "#5CC6BA"  # Rock flour, light: settled cells
GHOST = "#28424A"  # Unlit cells, large sizes only
INK = "#1D3138"
INK_DARK = "#E3EAE7"

# Grains settle toward the bed: 1, 1, 2, 3, 4, 5, 7 and 8 per row, placed by hand so no line
# or diagonal forms.
GRID = [
    ".....#..",
    ".#......",
    "...#...#",
    "#.#..#..",
    ".#.#.#.#",
    "#.#.##.#",
    "###.####",
    "########",
]


def lit(x: int, y: int) -> bool:
    return GRID[y][x] == "#"


def hexrgb(h: str) -> tuple[int, int, int]:
    return tuple(int(h[i : i + 2], 16) for i in (1, 3, 5))


# Geometry in units of a 10x10 tile: one unit of margin, 8x8 cells, gap 0.14 of a cell.
GAP = 0.14
RADIUS = 1.6


def mark_svg(size: int = 160) -> str:
    u = size / 10
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">',
        f'<rect width="{size}" height="{size}" rx="{RADIUS * u:.2f}" fill="{DEEP}"/>',
    ]
    for y in range(8):
        for x in range(8):
            fill = FLOUR if lit(x, y) else GHOST
            s = u * (1 - GAP)
            parts.append(f'<rect x="{u * (1 + x) + u * GAP / 2:.2f}" y="{u * (1 + y) + u * GAP / 2:.2f}" width="{s:.2f}" height="{s:.2f}" fill="{fill}"/>')
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def mark_large(size: int) -> Image.Image:
    big = 1024
    u = big / 10
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, big - 1, big - 1), radius=RADIUS * u, fill=hexrgb(DEEP))
    for y in range(8):
        for x in range(8):
            x0 = u * (1 + x) + u * GAP / 2
            y0 = u * (1 + y) + u * GAP / 2
            s = u * (1 - GAP)
            d.rectangle((x0, y0, x0 + s - 1, y0 + s - 1), fill=hexrgb(FLOUR if lit(x, y) else GHOST))
    return im.resize((size, size), Image.LANCZOS)


def mark_small(size: int) -> Image.Image:
    """Whole pixels: the 8x8 grid fills the tile, no margin, no ghost cells, square corners."""
    c = size // 8
    im = Image.new("RGBA", (size, size), hexrgb(DEEP) + (255,))
    px = im.load()
    for y in range(8):
        for x in range(8):
            if lit(x, y):
                for dy in range(c):
                    for dx in range(c):
                        px[x * c + dx, y * c + dy] = hexrgb(FLOUR) + (255,)
    return im


def mark(size: int) -> Image.Image:
    return mark_small(size) if size <= 32 else mark_large(size)


def wordmark_svg(text_color: str) -> str:
    font = TTFont(FONT)
    inst = instantiateVariableFont(font, {"wght": 760, "wdth": 125})
    glyphs = inst.getGlyphSet()
    cmap = inst.getBestCmap()
    upm = inst["head"].unitsPerEm
    asc = inst["hhea"].ascent
    cap = inst["OS/2"].sCapHeight
    x = 0
    paths = []
    for ch in "silt":
        name = cmap[ord(ch)]
        pen = SVGPathPen(glyphs)
        glyphs[name].draw(pen)
        paths.append(f'<path transform="translate({x},{asc}) scale(1,-1)" d="{pen.getCommands()}"/>')
        x += glyphs[name].width - upm * 0.02  # a touch of negative tracking at display size
    text_w = x
    # The mark sits centred on the cap height, a little taller than it.
    mark_size = cap * 1.18
    gap = upm * 0.22
    total_w = mark_size + gap + text_w
    height = asc + upm * 0.05
    u = mark_size / 10
    top = asc - cap - (mark_size - cap) / 2
    cells = []
    for yy in range(8):
        for xx in range(8):
            s = u * (1 - GAP)
            fill = FLOUR if lit(xx, yy) else GHOST
            cells.append(f'<rect x="{u * (1 + xx) + u * GAP / 2:.1f}" y="{top + u * (1 + yy) + u * GAP / 2:.1f}" width="{s:.1f}" height="{s:.1f}" fill="{fill}"/>')
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_w:.0f} {height:.0f}" height="96">\n'
        f'<rect x="0" y="{top:.1f}" width="{mark_size:.1f}" height="{mark_size:.1f}" rx="{RADIUS * u:.1f}" fill="{DEEP}"/>\n'
        + "\n".join(cells)
        + f'\n<g fill="{text_color}" transform="translate({mark_size + gap:.0f},0)">\n'
        + "\n".join(paths)
        + "\n</g>\n</svg>\n"
    )


def main() -> None:
    (HERE / "mark.svg").write_text(mark_svg(), encoding="utf-8")
    (HERE / "wordmark.svg").write_text(wordmark_svg(INK), encoding="utf-8")
    (HERE / "wordmark-dark.svg").write_text(wordmark_svg(INK_DARK), encoding="utf-8")
    png = HERE / "png"
    png.mkdir(exist_ok=True)
    for s in (16, 24, 32, 48, 64, 128, 256, 512):
        mark(s).save(png / f"silt-{s}.png")
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [mark(s) for s in ico_sizes]
    images[-1].save(HERE / "silt.ico", format="ICO", sizes=[(s, s) for s in ico_sizes], append_images=images[:-1])
    print("wrote mark.svg, wordmark.svg, wordmark-dark.svg, png/, silt.ico")


if __name__ == "__main__":
    main()
