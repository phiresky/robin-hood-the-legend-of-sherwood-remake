"""Source-camera painter preview of keep_north specs (diagnostic, no Blender).

Draws spec faces with the 35-degree source projection, pixel (x, y - z), beside
and over the untouched source crop.  Painter ordering by face-centroid depth is
approximate; the Blender packet remains the authoritative render.
"""
import math
from pathlib import Path
from PIL import Image, ImageDraw
from keep_north_geom import SIN, COS, world

ROOT = Path(__file__).resolve().parents[3]
REF = ROOT / 'level-editor/work/needs-review/lincoln-refinement/source-states'
TOWARD = (0.0, -COS, SIN)
SUN = (0.5712401270866394, -0.3459553122520447, 0.7443115711212158)


def _normal(ws):
    nx = ny = nz = 0.0
    for i in range(len(ws)):
        a, b = ws[i], ws[(i + 1) % len(ws)]
        nx += (a[1] - b[1]) * (a[2] + b[2])
        ny += (a[2] - b[2]) * (a[0] + b[0])
        nz += (a[0] - b[0]) * (a[1] + b[1])
    L = math.sqrt(nx * nx + ny * ny + nz * nz) or 1.0
    return (nx / L, ny / L, nz / L)


def _faces(meshes):
    out = []
    for node, mesh in meshes.items():
        for shell in mesh.shells:
            # Cells wind consistently; the signed volume says whether outward.
            vol = 0.0
            for f in shell.faces.values():
                ws = [world(p) for p in f]
                for i in range(1, len(ws) - 1):
                    a, b, c = ws[0], ws[i], ws[i + 1]
                    vol += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            sign = 1.0 if vol >= 0 else -1.0
            for f in shell.faces.values():
                ws = [world(p) for p in f]
                n = tuple(sign * v for v in _normal(ws))
                out.append((node, f, ws, n))
    return out


def render(meshes, box, path, scale=2, highlight=None):
    x0, y0, x1, y1 = box
    src = Image.open(REF / 'covered.png').crop(box)
    W, H = (x1 - x0) * scale, (y1 - y0) * scale
    src = src.resize((W, H), Image.NEAREST)
    solid = Image.new('RGB', (W, H), (0, 0, 0))
    d = ImageDraw.Draw(solid)
    faces = _faces(meshes)
    faces.sort(key=lambda r: sum(w[1] * TOWARD[1] + w[2] * TOWARD[2] for w in r[2]) / len(r[2]))
    over = src.copy()
    od = ImageDraw.Draw(over)
    for node, f, ws, n in faces:
        if n[0] * TOWARD[0] + n[1] * TOWARD[1] + n[2] * TOWARD[2] <= 0:
            continue
        lit = max(0.0, sum(n[i] * SUN[i] for i in range(3)))
        g = int(255 * min(1.0, 0.25 + 0.7 * lit))
        col = (g, g, g) if not highlight or node not in highlight else (g, int(g * .7), int(g * .7))
        poly = [((p[0] - x0) * scale, (p[1] - p[2] - y0) * scale) for p in f]
        d.polygon(poly, fill=col, outline=(40, 40, 40))
        od.line(poly + [poly[0]], fill=(0, 255, 255), width=1)
    sheet = Image.new('RGB', (W * 3 + 20, H), (255, 0, 255))
    sheet.paste(src, (0, 0))
    sheet.paste(solid, (W + 10, 0))
    sheet.paste(over, (2 * W + 20, 0))
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path)
    return path
