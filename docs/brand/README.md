# Silt brand

Silt is the fine sediment water leaves behind, layer on layer. A drive fills the same way. The
app shows the drive as a sediment core: a bar of cells, one colour per kind of thing, dense where
the bytes stay and sparse where they can be washed out.

Everything here follows from that one picture. The bar is the product; the rest of the
interface stays quiet so it can speak.

## Mark

![mark](png/silt-128.png)

An 8x8 grid of grains settling toward the bed: 1, 1, 2, 3, 4, 5, 7 and 8 grains per row, placed
by hand so no line or diagonal forms. Rock flour grains on a deep water tile.

| File | Use |
|---|---|
| `mark.svg` | the mark, any size from 48 px up |
| `png/silt-{16,24,32}.png` | small sizes, drawn on whole pixels: no margin, no unlit cells |
| `png/silt-{48..512}.png` | larger sizes, with margin and unlit cells |
| `silt.ico` | the app icon (16 to 256) |
| `wordmark.svg`, `wordmark-dark.svg` | mark + "silt" for light and dark backgrounds |

`make.py` builds all of them: `uv run --with pillow --with fonttools docs/brand/make.py`.

Keep one cell of clear space around the mark. Don't recolour the grains, rotate the tile, or
redraw the grid with a different pattern.

## Colour

The interface follows the Windows light or dark setting.

| Token | Light | Dark | Use |
|---|---|---|---|
| `bed` | `#EEF1EE` Fog | `#142024` Riverbed | window background |
| `surface` | `#F8FAF8` | `#1B2A2F` | menus, dialogs, the selected row |
| `ink` | `#1D3138` Deep water | `#E3EAE7` | text |
| `ink-2` | `#56676C` Bank | `#93A3A5` | secondary text, paths |
| `line` | `#C9D1CF` | `#2A3A3F` | separators, the outline of inputs |
| `free` | `#D5DCD9` | `#22343A` | unlit cells: free space in the bar |
| `flour` | `#1E8A86` Rock flour | `#4CC0B5` | actions, focus, progress |
| `danger` | `#B3412F` | `#E0705C` | the confirm button of Clean, Trash and Empty Recycle Bin |

Rock flour is the turquoise glacial silt gives a river. It marks what you can do, never a
category, so it never appears in the bar.

### Categories

Eleven hues spread around the wheel; neutrals for what you can't act on. The same values work on
both backgrounds.

| Category | Hex |
|---|---|
| Apps | `#3F78B0` |
| Games | `#7D5BC4` |
| Developer | `#5A9A48` |
| Caches | `#D2A23C` |
| Documents | `#D9773F` |
| Photos | `#C9518A` |
| Videos | `#C9473E` |
| Music | `#3BA3C4` |
| Other | `#A69A7C` |
| System | `#7D8A8E` |
| Trash | `#A9B2B0` |

Colour is never the only signal: the legend names every segment and its size, and hovering a
cell names its category.

## Type

One family: [Archivo](https://github.com/Omnibus-Type/Archivo) (SIL Open Font License),
variable in weight (100 to 900) and width (62 to 125). It ships in the app
(`crates/silt-app/assets/fonts`).

Width carries meaning. Totals are set wide; context and long paths are set narrow. When Shrink
finishes, the saved figure is the one set narrow: the type does what the files did.

| Role | Size / line | Weight | Width | Use |
|---|---|---|---|---|
| Display | 56 / 60 | 720 | 125 | the used figure on Overview |
| Display context | 20 / 24 | 450 | 75 | "of 299 GB", beside Display |
| Title | 20 / 26 | 640 | 112 | screen titles |
| Body | 14 / 20 | 420 | 100 | text, rows |
| Label | 13 / 18 | 560 | 100 | buttons, legend names |
| Figure | 14 / 20 | 520 | 92 | sizes in tables |
| Path | 13 / 18 | 420 | 75 | file and folder paths |

Every figure uses tabular numerals (`'tnum' 1`) so columns line up and counting doesn't jitter.
Sentence case everywhere. No all caps, no letter-spaced labels, no monospace.

## The bar

The bar is drawn as pixels (`rune::Pixels`), cell by cell, so the window and the headless
snapshots match exactly.

**Grid.** Cells are 4 px squares on a 5 px pitch at 100 % scale; at other scales the pitch is
`round(5 x scale)` device pixels and the gap `max(1, round(scale))`. The Overview bar is 18 rows
high and as many columns as fit. Column `c` covers the bytes from `c / columns` to
`(c + 1) / columns` of the drive's capacity.

**Order.** Segments run in a fixed order so they never swap places between scans or drives:
Apps, Games, Developer, Caches, Documents, Photos, Videos, Music, Other, System, Trash, then free
space.

**Density.** Within a segment, the bytes that stay come first and are fully lit. The part Clean
and Shrink can recover comes last, lit at 25 %: a cell is lit when its threshold in the 8x8
Bayer matrix below is under 16. Free space is the grid unlit, in `free`.

**Seams.** Where two segments meet, three columns blend: a cell takes the left colour when its
Bayer threshold is under `64 x (1 - t)`, where `t` runs from 0 to 1 across the seam. Segments
narrower than one column still get one column.

```
 0 32  8 40  2 34 10 42
48 16 56 24 50 18 58 26
12 44  4 36 14 46  6 38
60 28 52 20 62 30 54 22
 3 35 11 43  1 33  9 41
51 19 59 27 49 17 57 25
15 47  7 39 13 45  5 37
63 31 55 23 61 29 53 21
```

## Motion

One moment belongs to the brand: **settling.** While a scan runs, cells light up in `ink-2`
from the left as bytes are found. The leading edge is eight columns wide and fills by Bayer
threshold, so it arrives as grain, not as a wipe. When the scan is classified, each cell takes
its category colour over 240 ms.

Everything else answers the person:

- Hovering Shrink or Clean in the Reclaim list turns off the cells it would recover (120 ms).
- After an action, segments narrow to their new width on a spring (stiffness 220, damping 26).
- With reduced motion on, the scan lights the bar in four steps and changes happen at once.

## Words

Short, plain, from the user's side. An action keeps its name from button to result.

| Action | Button | Result |
|---|---|---|
| Shrink | Shrink 3 folders | Shrunk 3 folders, saved 2.8 GB |
| Restore | Restore | Restored, 2.8 GB back as it was |
| Clean | Clean 4 caches | Cleaned 4 caches, freed 9.1 GB |
| Trash | Move to Recycle Bin | Moved 12 items to the Recycle Bin |
| Empty Recycle Bin | Empty Recycle Bin | Emptied the Recycle Bin, freed 4.2 GB |

Errors say what happened and what to do: "12 files were in use and stayed. Close the app that
uses them and run Clean again." No apologies, no exclamation marks.
