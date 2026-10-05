"""Private analytic fit of closed kindling sticks; no worker mutation."""
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy.optimize import minimize

OUT = Path(__file__).resolve().parents[3] / 'level-editor/work/croisement02-refinement'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
BOX = (130, 940, 198, 1020)


def geometry(parameters):
    """Closed stick volumes; rear stick count and placement are inferred."""
    xr, yr, height, taper, cx, cy = parameters
    vertices, faces = [], []
    for ring, count in [(1., 18), (.5, 9), (0., 1)]:
        for j in range(count):
            angle = j * math.tau / count + .13 * ring
            radial = np.array((xr * math.cos(angle), yr * math.sin(angle), 0.)) * ring
            base = np.array((cx, -cy / SIN, 1.7)) + radial
            top = np.array((cx - 1.8, -cy / SIN + 3., height + 1.2 * math.sin(3 * angle))) + radial * taper
            axis = top - base
            axis /= np.linalg.norm(axis)
            u = np.cross(axis, (1., 0., 0.)); u /= np.linalg.norm(u)
            v = np.cross(axis, u)
            start = len(vertices)
            for center, radius in [(base, 1.75), (top, 1.25)]:
                vertices.extend(center + radius * (u * math.cos(k * math.tau / 10) + v * math.sin(k * math.tau / 10)) for k in range(10))
            faces.append(tuple(start + k for k in reversed(range(10))))
            faces.extend((start + k, start + (k + 1) % 10, start + 10 + (k + 1) % 10, start + 10 + k) for k in range(10))
            faces.append(tuple(start + 10 + k for k in range(10)))
    return np.asarray(vertices), faces


def main():
    dest = OUT / 'restart2-vegetation/kindling-outline-research'
    dest.mkdir(exist_ok=True)
    mask_path = OUT / 'baseline/masks/000104.png'
    crop = np.asarray(Image.open(mask_path).convert('L')) > 0
    target = np.zeros((BOX[3] - BOX[1], BOX[2] - BOX[0]), bool)
    target[958 - BOX[1]:958 - BOX[1] + crop.shape[0], 148 - BOX[0]:148 - BOX[0] + crop.shape[1]] = crop

    def raster(parameters):
        points, faces = geometry(parameters)
        screen = np.column_stack((points[:, 0] - BOX[0], -points[:, 1] * SIN - points[:, 2] * COS - BOX[1]))
        image = Image.new('L', (target.shape[1] * 3, target.shape[0] * 3)); draw = ImageDraw.Draw(image)
        for face in faces:
            draw.polygon([tuple(screen[i] * 3) for i in face], fill=255)
        return np.asarray(image)[1::3, 1::3] > 0

    def scores(mask):
        inside = int((mask & target).sum()); extra = int((mask & ~target).sum()); missing = int((~mask & target).sum())
        return dict(inside=inside, extra=extra, missing=missing, iou=inside / (inside + extra + missing))

    def loss(parameters):
        score = scores(raster(parameters))
        return score['missing'] + 1.4 * score['extra']

    initial = np.array((15., 15., 38.314, .35, 168., 1004.))
    fitted = minimize(loss, initial, method='Powell', bounds=[(12, 19), (12, 26), (34, 53), (.22, .52), (162, 172), (999, 1008)], options=dict(maxiter=6, maxfev=1000, xtol=.08, ftol=.001))
    mask = raster(fitted.x)
    source_path = OUT / 'animation-references/composite-frame-0.png'
    overlay = np.array(Image.open(source_path).convert('RGBA').crop(BOX))
    overlay[target & ~mask, :3] = (255, 0, 120)
    overlay[mask & ~target, :3] = (0, 170, 255)
    Image.fromarray(overlay).resize((340, 400), Image.Resampling.NEAREST).save(dest / 'source-residual.png')
    points, faces = geometry(fitted.x)
    report = dict(status='Private analytic hypothesis, not a model or approval', parameter_order=['x_radius', 'y_radius', 'height', 'top_taper', 'center_x', 'ground_center_source_y'], initial=initial.tolist(), fitted=fitted.x.tolist(), before=scores(raster(initial)), after=scores(mask), source_box=BOX, source_sha256=hashlib.sha256(source_path.read_bytes()).hexdigest(), mask_sha256=hashlib.sha256(mask_path.read_bytes()).hexdigest(), vertices=points.tolist(), faces=faces, limitations=['Stick count, rear arrangement and depth are inferred; source mask unchanged.', 'Binding bands and visible cut caps require artwork review before model construction.', 'Requires native material projection, all eight views and physical ground/neighbor checks.'])
    (dest / 'fit.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ['before', 'after', 'fitted']}))


if __name__ == '__main__':
    main()
