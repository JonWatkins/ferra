"""Draws ferra's splash-screen logo (block letters with double-line
shadows) as an SVG of shapes, so it looks the same in any font."""
import re, sys

src = open(sys.argv[1]).read()
block = src[src.index("pub const LOGO"):]
block = block[:block.index("];")]
rows = re.findall(r'"([^"]*)"', block)
W, H = 12, 24            # one character cell
cols = max(len(r) for r in rows)
fill, shadow = "#7aa2f7", "#3d59a1"
d = 2.5                   # half the gap between a double line's strokes
parts = []
for y, row in enumerate(rows):
    for x, c in enumerate(row):
        x0, y0 = x * W, y * H
        x1, y1 = x0 + W, y0 + H
        cx, cy = x0 + W / 2, y0 + H / 2
        lines = []
        if c == "█":
            parts.append(f'<rect x="{x0}" y="{y0}" width="{W + 0.4}" height="{H + 0.4}" fill="{fill}"/>')
            continue
        if c == "═":
            lines = [[(x0, cy - d), (x1, cy - d)], [(x0, cy + d), (x1, cy + d)]]
        elif c == "║":
            lines = [[(cx - d, y0), (cx - d, y1)], [(cx + d, y0), (cx + d, y1)]]
        elif c == "╗":
            lines = [[(x0, cy - d), (cx + d, cy - d), (cx + d, y1)], [(x0, cy + d), (cx - d, cy + d), (cx - d, y1)]]
        elif c == "╔":
            lines = [[(x1, cy - d), (cx - d, cy - d), (cx - d, y1)], [(x1, cy + d), (cx + d, cy + d), (cx + d, y1)]]
        elif c == "╝":
            lines = [[(x0, cy + d), (cx + d, cy + d), (cx + d, y0)], [(x0, cy - d), (cx - d, cy - d), (cx - d, y0)]]
        elif c == "╚":
            lines = [[(x1, cy + d), (cx - d, cy + d), (cx - d, y0)], [(x1, cy - d), (cx + d, cy - d), (cx + d, y0)]]
        elif c != " ":
            raise SystemExit(f"no shape for {c!r}")
        for line in lines:
            pts = " ".join(f"{px:g},{py:g}" for px, py in line)
            parts.append(f'<polyline points="{pts}"/>')

width, height = cols * W, len(rows) * H
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="-6 -6 {width + 12} {height + 12}" width="{width + 12}" height="{height + 12}" role="img" aria-label="ferra">
<title>ferra</title>
<g fill="none" stroke="{shadow}" stroke-width="1.6" stroke-linecap="butt" stroke-linejoin="miter">
{chr(10).join(p for p in parts if p.startswith("<polyline"))}
</g>
{chr(10).join(p for p in parts if p.startswith("<rect"))}
</svg>
'''
open(sys.argv[2], "w").write(svg)
print(f"{len(rows)} rows, {cols} columns, {len(parts)} shapes")
