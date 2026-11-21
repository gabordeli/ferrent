"""Builds the Ferrent logo SVG: a mark plus the wordmark as outlines.

The wordmark is converted from the bundled IBM Plex Sans SemiBold face so the
asset needs no font at runtime (and no external design tool).
"""

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.ttLib import TTFont

FONT = "crates/gui/assets/fonts/IBMPlexSans-SemiBold.ttf"
TEXT = "Ferrent"
SIZE = 26.0
MARK = 30.0
GAP = 10.0
BASELINE = 24.0

font = TTFont(FONT)
glyphs = font.getGlyphSet()
cmap = font.getBestCmap()
upm = font["head"].unitsPerEm
scale = SIZE / upm

parts = []
x = MARK + GAP
for char in TEXT:
    name = cmap[ord(char)]
    pen = SVGPathPen(glyphs)
    glyphs[name].draw(pen)
    parts.append(
        f'<path transform="translate({x:.3f},{BASELINE:.3f}) scale({scale:.6f},{-scale:.6f})" '
        f'd="{pen.getCommands()}"/>'
    )
    x += glyphs[name].width * scale

width = x - GAP + 2.0
svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="32" viewBox="0 0 {width:.0f} 32">
  <rect x="0" y="1" width="{MARK}" height="{MARK}" rx="7" fill="#1677F2"/>
  <path d="M15 8.2v10.2M10.6 14.4 15 18.8l4.4-4.4" fill="none" stroke="#F8FAFC"
        stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M10.8 22.4h8.4" fill="none" stroke="#F8FAFC" stroke-width="2.6"
        stroke-linecap="round"/>
  <g fill="#F8FAFC">
    {chr(10).join('    ' + part for part in parts)}
  </g>
</svg>
"""
open("crates/gui/assets/logo.svg", "w").write(svg)

mark = """<svg xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 30 30">
  <rect width="30" height="30" rx="7" fill="#1677F2"/>
  <path d="M15 8.2v10.2M10.6 14.4 15 18.8l4.4-4.4" fill="none" stroke="#F8FAFC"
        stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M10.8 22.4h8.4" fill="none" stroke="#F8FAFC" stroke-width="2.6"
        stroke-linecap="round"/>
</svg>
"""
open("crates/gui/assets/mark.svg", "w").write(mark)
print(f"logo {width:.0f}x32, mark 30x30")
