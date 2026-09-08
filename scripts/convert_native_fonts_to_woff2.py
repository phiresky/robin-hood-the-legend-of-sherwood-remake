#!/usr/bin/env python3
"""Convert Robin Hood SBFONT bitmaps losslessly into color WOFF2 fonts."""

from __future__ import annotations

import argparse
import bz2
import io
import json
import re
import struct
import zlib
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables.BitmapGlyphMetrics import SmallGlyphMetrics
from fontTools.ttLib.tables.C_B_D_T_ import cbdt_bitmap_format_17
from fontTools.ttLib.tables.E_B_L_C_ import (
    SbitLineMetrics,
    Strike,
    eblc_index_sub_table_1,
)
from fontTools.ttLib.tables.S_V_G_ import SVGDocument
from PIL import Image


@dataclass(frozen=True)
class Character:
    codepoint: int
    start: int
    width: int
    pre_spacing: int
    post_spacing: int


@dataclass(frozen=True)
class NativeFont:
    name: str
    height: int
    baseline: int
    extra_spacing: int
    characters: tuple[Character, ...]
    glyph_width: int
    glyph_pixels: tuple[int, ...]
    alpha_width: int
    alpha_pixels: tuple[int, ...]
    uses_glyph_mask: bool


class Reader:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.offset = 0

    def take(self, size: int) -> bytes:
        end = self.offset + size
        if end > len(self.data):
            raise ValueError(f"unexpected EOF at {self.offset}, wanted {size} bytes")
        value = self.data[self.offset:end]
        self.offset = end
        return value

    def unpack(self, fmt: str):
        size = struct.calcsize(fmt)
        return struct.unpack(fmt, self.take(size))


def read_picture(reader: Reader) -> tuple[int, int, tuple[int, ...]]:
    width, height, packing, packed_size = reader.unpack("<HHII")
    payload = reader.take(packed_size)
    if packing == 0:
        raw = payload
    elif packing == 1:
        raw = zlib.decompress(payload)
    elif packing == 2:
        raw = bz2.decompress(payload)
    else:
        raise ValueError(f"unsupported Sixteen packing value {packing}")
    expected = width * height * 2
    if len(raw) != expected:
        raise ValueError(f"picture is {len(raw)} bytes, expected {expected}")
    return width, height, struct.unpack(f"<{width * height}H", raw)


def parse_native_font(path: Path) -> NativeFont:
    reader = Reader(path.read_bytes())
    if reader.take(6) != b"SBFONT":
        raise ValueError(f"{path}: not an SBFONT file")
    (version,) = reader.unpack("<I")
    name = reader.take(32).split(b"\0", 1)[0].decode("latin-1")
    _flags, _styles, height, _cell_width, baseline, count = reader.unpack("<IIIIII")
    extra_spacing = reader.unpack("<i")[0] if version == 0x0200 else 0
    characters = tuple(
        Character(*reader.unpack("<HIIii")) for _ in range(count)
    )
    glyph_width, glyph_height, glyph_pixels = read_picture(reader)
    alpha_width, alpha_height, alpha_pixels = read_picture(reader)
    if glyph_height != height or alpha_height != height:
        raise ValueError(
            f"{path}: atlas heights {glyph_height}/{alpha_height} != font height {height}"
        )
    # Most fonts carry the silhouette in the colour atlas: each scanline has
    # a flat background colour and glyph pixels differ from it. The briefing
    # fonts use a flat colour atlas and carry their silhouette in alpha only.
    uses_glyph_mask = any(
        glyph_pixels[y * glyph_width + x] != glyph_pixels[y * glyph_width]
        for y in range(height)
        for x in range(glyph_width)
    )
    return NativeFont(
        name=name,
        height=height,
        baseline=baseline,
        extra_spacing=extra_spacing,
        characters=characters,
        glyph_width=glyph_width,
        glyph_pixels=glyph_pixels,
        alpha_width=alpha_width,
        alpha_pixels=alpha_pixels,
        uses_glyph_mask=uses_glyph_mask,
    )


def safe_family_name(stem: str) -> str:
    words = re.sub(r"([a-z])([A-Z])", r"\1 \2", stem).replace("_", " ")
    return f"Robin Hood {words}"


def glyph_name(codepoint: int) -> str:
    return f"uni{codepoint:04X}" if codepoint <= 0xFFFF else f"u{codepoint:06X}"


def pixel_is_visible(font: NativeFont, x: int, y: int) -> bool:
    glyph = font.glyph_pixels[y * font.glyph_width + x]
    alpha = font.alpha_pixels[y * font.alpha_width + x] & 0x1F
    if font.uses_glyph_mask:
        row_background = font.glyph_pixels[y * font.glyph_width]
        return glyph != row_background
    return alpha != 0


def pixel_rgba(font: NativeFont, x: int, y: int) -> tuple[int, int, int, int]:
    glyph = font.glyph_pixels[y * font.glyph_width + x]
    alpha = (font.alpha_pixels[y * font.alpha_width + x] & 0x1F) << 3
    if glyph == 0x07C0:
        alpha = 0
    return (
        (glyph >> 8) & 0xF8,
        (glyph >> 3) & 0xFC,
        (glyph << 3) & 0xF8,
        alpha,
    )


def make_glyph(font: NativeFont, char: Character, scale: int):
    pen = TTGlyphPen(None)
    # The outline is only a sanitizer-compatible monochrome fallback. Color
    # glyph placement owns the native pre-spacing, so its x-min stays at zero.
    left = 0
    for y in range(font.height):
        run_start = None
        for local_x in range(char.width + 1):
            visible = (
                local_x < char.width
                and pixel_is_visible(font, char.start + local_x, y)
            )
            if visible and run_start is None:
                run_start = local_x
            elif not visible and run_start is not None:
                x0 = (left + run_start) * scale
                x1 = (left + local_x) * scale
                y0 = (font.baseline - y - 1) * scale
                y1 = (font.baseline - y) * scale
                pen.moveTo((x0, y0))
                pen.lineTo((x0, y1))
                pen.lineTo((x1, y1))
                pen.lineTo((x1, y0))
                pen.closePath()
                run_start = None
    return pen.glyph()


def glyph_png(font: NativeFont, char: Character, bitmap_scale: int = 1) -> bytes | None:
    rgba = bytes(
        channel
        for y in range(font.height)
        for x in range(char.width)
        for channel in pixel_rgba(font, char.start + x, y)
    )
    if not any(rgba[index] for index in range(3, len(rgba), 4)):
        return None
    image = Image.frombytes("RGBA", (char.width, font.height), rgba)
    if bitmap_scale != 1:
        image = image.resize(
            (char.width * bitmap_scale, font.height * bitmap_scale),
            Image.Resampling.NEAREST,
        )
    output = io.BytesIO()
    image.save(output, format="PNG", optimize=False, compress_level=9)
    return output.getvalue()


def dominant_color(font: NativeFont) -> str:
    colors: Counter[tuple[int, int, int]] = Counter()
    for index, (glyph, alpha) in enumerate(zip(font.glyph_pixels, font.alpha_pixels)):
        y, x = divmod(index, font.glyph_width)
        if not pixel_is_visible(font, x, y):
            continue
        rgb = (
            (glyph >> 8) & 0xF8,
            (glyph >> 3) & 0xFC,
            (glyph << 3) & 0xF8,
        )
        colors[rgb] += 1
    if not colors:
        return "#f4e8c7"
    red, green, blue = colors.most_common(1)[0][0]
    return f"#{red:02x}{green:02x}{blue:02x}"


def convert(path: Path, output: Path) -> dict[str, object]:
    native = parse_native_font(path)
    scale = 64
    units_per_em = native.height * scale
    family = safe_family_name(path.stem)

    chars = {char.codepoint: char for char in native.characters}
    order = [".notdef"] + [glyph_name(codepoint) for codepoint in sorted(chars)]
    cmap = {codepoint: glyph_name(codepoint) for codepoint in sorted(chars)}

    empty_pen = TTGlyphPen(None)
    glyphs = {".notdef": empty_pen.glyph()}
    metrics = {".notdef": (max(scale * 4, units_per_em // 2), 0)}
    for codepoint, char in sorted(chars.items()):
        name = glyph_name(codepoint)
        # The native pre-spacing is encoded in CBDT BearingX and in the SVG
        # pixel coordinates below. Keep the fallback outline and hmtx LSB at
        # zero: putting it in hmtx too makes some browser color-font paths
        # apply the bearing twice (most visibly dialog.fnt's negative `g`).
        glyphs[name] = empty_pen.glyph() if codepoint == 0x20 else make_glyph(native, char, scale)
        advance = max(
            scale,
            (char.pre_spacing + char.width + char.post_spacing + native.extra_spacing)
            * scale,
        )
        metrics[name] = (advance, 0)

    builder = FontBuilder(units_per_em, isTTF=True)
    builder.setupGlyphOrder(order)
    builder.setupCharacterMap(cmap)
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics(metrics)
    ascender = native.baseline * scale
    descender = -(native.height - native.baseline) * scale
    builder.setupHorizontalHeader(ascent=ascender, descent=descender, lineGap=0)
    builder.setupNameTable(
        {
            "familyName": family,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"RobinHoodWebfonts:{path.stem}:1.0",
            "fullName": family,
            "psName": family.replace(" ", "-"),
            "version": "Version 1.0",
        }
    )
    builder.setupOS2(
        sTypoAscender=ascender,
        sTypoDescender=descender,
        sTypoLineGap=0,
        usWinAscent=max(0, ascender),
        usWinDescent=max(0, -descender),
    )
    builder.setupPost()
    builder.setupMaxp()
    svg_chars: dict[str, tuple[Character, int]] = {}
    embedded_png_glyphs = 0
    for codepoint, char in sorted(chars.items()):
        # NativeFont::layout_quads deliberately advances over spaces without
        # drawing their atlas cell. Some files contain stray pixels there.
        if codepoint == 0x20:
            continue
        png = glyph_png(native, char)
        if png is None:
            continue
        svg_chars[glyph_name(codepoint)] = (char, char.pre_spacing)
        embedded_png_glyphs += 1

    cblc = newTable("CBLC")
    cblc.version = 3.0
    cbdt = newTable("CBDT")
    cbdt.version = 3.0
    cblc.strikes = []
    cbdt.strikeData = []
    for bitmap_scale in range(1, 5):
        bitmap_glyphs = {}
        for name, (char, bearing_x) in svg_chars.items():
            glyph = cbdt_bitmap_format_17(None, builder.font)
            glyph.imageData = glyph_png(native, char, bitmap_scale)
            glyph.metrics = SmallGlyphMetrics()
            glyph.metrics.height = native.height * bitmap_scale
            glyph.metrics.width = char.width * bitmap_scale
            glyph.metrics.BearingX = bearing_x * bitmap_scale
            glyph.metrics.BearingY = native.baseline * bitmap_scale
            glyph.metrics.Advance = max(
                1,
                char.pre_spacing
                + char.width
                + char.post_spacing
                + native.extra_spacing,
            ) * bitmap_scale
            bitmap_glyphs[name] = glyph

        strike = Strike()
        size = strike.bitmapSizeTable
        size.colorRef = 0
        size.ppemX = native.height * bitmap_scale
        size.ppemY = native.height * bitmap_scale
        size.bitDepth = 32
        size.flags = 1
        size.hori = make_line_metrics(
            ascender=native.baseline * bitmap_scale,
            descender=(native.baseline - native.height) * bitmap_scale,
            width_max=max(char.width for char in chars.values()) * bitmap_scale,
        )
        size.vert = make_line_metrics(ascender=0, descender=0, width_max=0)
        index = eblc_index_sub_table_1(None, builder.font)
        index.indexFormat = 1
        index.imageFormat = 17
        index.names = [name for name in order if name in bitmap_glyphs]
        index.locations = []
        strike.indexSubTables = [index]
        cblc.strikes.append(strike)
        cbdt.strikeData.append(bitmap_glyphs)

    builder.font["CBDT"] = cbdt
    builder.font["CBLC"] = cblc

    # Firefox currently ignores CBDT/CBLC in web-loaded WOFF2 fonts. Include
    # the same source pixels as SVG rectangles for a second lossless color
    # glyph representation; Chromium selects CBDT, Firefox selects SVG.
    svg = newTable("SVG ")
    svg.docList = []
    glyph_map = builder.font.getReverseGlyphMap()
    for name, (char, bearing_x) in svg_chars.items():
        gid = glyph_map[name]
        document = svg_glyph_document(native, char, scale, gid, bearing_x)
        svg.docList.append(SVGDocument(document, gid, gid, compressed=True))
    builder.font["SVG "] = svg
    builder.font.flavor = "woff2"
    output.parent.mkdir(parents=True, exist_ok=True)
    builder.save(output)

    return {
        "source": path.name,
        "file": output.name,
        "family": family,
        "internal_name": native.name,
        "height": native.height,
        "baseline": native.baseline,
        "glyphs": len(chars),
        "embedded_png_glyphs": embedded_png_glyphs,
        "color": dominant_color(native),
    }


def make_line_metrics(ascender: int, descender: int, width_max: int) -> SbitLineMetrics:
    metrics = SbitLineMetrics()
    metrics.ascender = ascender
    metrics.descender = descender
    metrics.widthMax = width_max
    metrics.caretSlopeNumerator = 1
    metrics.caretSlopeDenominator = 0
    metrics.caretOffset = 0
    metrics.minOriginSB = 0
    metrics.minAdvanceSB = 0
    metrics.maxBeforeBL = ascender
    metrics.minAfterBL = descender
    metrics.pad1 = 0
    metrics.pad2 = 0
    return metrics


def svg_glyph_document(
    font: NativeFont,
    char: Character,
    scale: int,
    glyph_id: int,
    bearing_x: int,
) -> str:
    rects = []
    for y in range(font.height):
        run_start = 0
        run_color = pixel_rgba(font, char.start, y)
        for x in range(1, char.width + 1):
            color = pixel_rgba(font, char.start + x, y) if x < char.width else None
            if color == run_color:
                continue
            red, green, blue, alpha = run_color
            if alpha:
                rects.append(
                    f'<rect x="{(bearing_x + run_start) * scale}" '
                    f'y="{(y - font.baseline) * scale}" '
                    f'width="{(x - run_start) * scale}" height="{scale}" '
                    f'fill="#{red:02x}{green:02x}{blue:02x}" '
                    f'fill-opacity="{alpha / 255:.9f}"/>'
                )
            run_start = x
            run_color = color
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" shape-rendering="crispEdges">'
        f'<g id="glyph{glyph_id}">'
        + "".join(rects)
        + '</g></svg>'
    )


def convert_truetype(path: Path, output: Path) -> dict[str, object]:
    font = TTFont(path)
    font.flavor = "woff2"
    output.parent.mkdir(parents=True, exist_ok=True)
    font.save(output)
    best_cmap = font.getBestCmap() or {}
    return {
        "source": path.name,
        "file": output.name,
        "family": "Arial",
        "internal_name": "Arial",
        "glyphs": len(best_cmap),
        "kind": "TrueType",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path, help="directory containing .bfn/.fnt files")
    parser.add_argument("output", type=Path, help="output directory for WOFF2 files")
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()

    sources = sorted((*args.input.glob("*.bfn"), *args.input.glob("*.fnt")))
    if not sources:
        raise SystemExit(f"no native font files found in {args.input}")
    manifest = []
    for source in sources:
        destination = args.output / f"{source.stem}.woff2"
        entry = convert(source, destination)
        entry["kind"] = "Native bitmap conversion"
        manifest.append(entry)
        print(f"{source.name} -> {destination.name}")
    arial = args.input / "arial.ttf"
    if arial.is_file():
        destination = args.output / "arial.woff2"
        manifest.append(convert_truetype(arial, destination))
        print(f"{arial.name} -> {destination.name}")
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
