"""Test a whole tipped wagon against frozen terminal artwork before rendering."""
import hashlib, json, math
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.optimize import differential_evolution
from catalog import OUT

SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
DEST = OUT / 'restart2-state/south-cart-body-fit-v1'
U = np.array([61., -21. / SIN, 0.]); LENGTH = np.linalg.norm(U); U /= LENGTH
V = np.array([-U[1], U[0], 0.]); Z = np.array([0., 0., 1.])
BOX = (945, 820, 1165, 1000)


def geometry(parameters):
    roll, halfwidth, eave, rise, end_extension, board_height = parameters
    angle = math.radians(roll)
    across = V * math.cos(angle) + Z * math.sin(angle)
    up = Z * math.cos(angle) - V * math.sin(angle)
    # Wheel faces rotate with the wagon. Their upper pair fixes native placement.
    wheel_radius = 21.
    minimum = -halfwidth * abs(math.sin(angle)) - wheel_radius * abs(math.cos(angle))
    top_z = halfwidth * math.sin(angle) - minimum
    upper = np.array([1059., -(844. + top_z * COS) / SIN, top_z])
    origin = upper - across * halfwidth
    pieces = []

    def convert(local):
        return [list(origin + U * a + across * b + up * c) for a, b, c in local]

    def box(name, ranges):
        (a0, a1), (b0, b1), (c0, c1) = ranges
        vertices = convert([(a, b, c) for c in [c0, c1] for b in [b0, b1] for a in [a0, a1]])
        faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]]
        pieces.append(dict(name=name, vertices=vertices, faces=faces))

    a0, a1 = -end_extension, LENGTH + end_extension
    box('Tipped wagon bed', [(a0, a1), (-halfwidth + 4, halfwidth - 4), (8, 12)])
    for side in [-1, 1]:
        b = side * (halfwidth - 4)
        box(f'Side board {side}', [(a0, a1), (b - 1.5, b + 1.5), (10, 10 + board_height)])
    for a in [a0, a1]:
        box(f'End board {a}', [(a - 1.5, a + 1.5), (-halfwidth + 4, halfwidth - 4), (10, 10 + board_height)])
    for a in [a0, a1]:
        for side in [-1, 1]:
            b = side * (halfwidth - 5)
            box(f'Canopy post {a} {side}', [(a - 1.5, a + 1.5), (b - 1.5, b + 1.5), (10, eave)])
    n = 12; local = []
    for a in [a0, a1]:
        for thickness in [0., -2.]:
            for i in range(n + 1):
                theta = math.pi * i / n
                local.append((a, (halfwidth - 3) * math.cos(theta), eave + rise * math.sin(theta) + thickness))
    stride = n + 1; faces = []
    for i in range(n):
        for a, b in [(0, stride * 2), (stride, stride * 3), (0, stride), (stride * 2, stride * 3)]:
            faces.append([a + i, a + i + 1, b + i + 1, b + i])
    for i in [0, n]: faces.append([i, stride + i, stride * 3 + i, stride * 2 + i])
    pieces.append(dict(name='Tipped barrel canopy shell', vertices=convert(local), faces=faces))
    return pieces, dict(roll_degrees=roll, wheelbase=LENGTH, upper_wheel_height=top_z, upper_wheel_native_centers=[[1059, 844], [1120, 865]], origin=origin.tolist(), across=across.tolist(), up=up.tolist())


def silhouette(pieces):
    im = Image.new('L', (BOX[2] - BOX[0], BOX[3] - BOX[1])); draw = ImageDraw.Draw(im)
    for piece in pieces:
        points = [(p[0] - BOX[0], -p[1] * SIN - p[2] * COS - BOX[1]) for p in piece['vertices']]
        for face in piece['faces']: draw.polygon([points[i] for i in face], fill=255)
    return np.asarray(im) > 0


def main():
    DEST.mkdir(exist_ok=False)
    src = OUT / 'state-target-evidence/south-cart/manifest.json'
    part = json.loads(src.read_text())['parts'][0]; frame = part['frames'][-1]
    image = Image.open(frame['image']).convert('RGBA')
    assert hashlib.sha256(Path(frame['image']).read_bytes()).hexdigest() == frame['image_sha256']
    left, top = [part['position'][i] + frame['offset'][i] for i in range(2)]
    native = Image.new('RGBA', (BOX[2] - BOX[0], BOX[3] - BOX[1])); native.alpha_composite(image, (int(left - BOX[0]), int(top - BOX[1])))
    # Body-only survey excludes ground scraps, detached wheels and living actors.
    polygon = [(54, 64), (72, 28), (96, 14), (129, 19), (145, 42), (138, 72), (128, 93), (107, 112), (65, 98)]
    domain = Image.new('L', native.size); ImageDraw.Draw(domain).polygon([(x + left - BOX[0], y + top - BOX[1]) for x, y in polygon], fill=255)
    expected = (np.asarray(domain) > 0) & (np.asarray(native)[:, :, 3] > 0)
    def objective(parameters):
        pieces, _ = geometry(parameters); mask = silhouette(pieces)
        return int((expected & ~mask).sum()) * 2 + int((mask & ~expected).sum())
    result = differential_evolution(objective, [(60, 120), (22, 42), (38, 90), (8, 30), (2, 15), (8, 26)], seed=734, maxiter=120, popsize=12, polish=False, workers=1)
    pieces, placement = geometry(result.x); mask = silhouette(pieces)
    missing = int((expected & ~mask).sum()); excess = int((mask & ~expected).sum())
    diagnostic = np.asarray(native).copy(); diagnostic[expected & ~mask] = [255, 40, 200, 255]; diagnostic[mask & ~expected] = [40, 180, 255, 255]
    Image.fromarray(diagnostic).resize((880, 720), Image.Resampling.NEAREST).save(DEST / 'native-fit.png')
    native.resize((880, 720), Image.Resampling.NEAREST).save(DEST / 'native-source.png')
    report = dict(status='Private rigid tipped-wagon hypothesis; inspect source mismatch before any render or acceptance', source_manifest_sha256=hashlib.sha256(src.read_bytes()).hexdigest(), source_frame=frame, body_domain_polygon=polygon, parameters=result.x.tolist(), placement=placement, expected_pixels=int(expected.sum()), missing_pixels=missing, excess_pixels=excess, objective=float(result.fun), pieces=pieces, limits=['Template tests a coherent wagon, not a claim that intact rigid geometry survived collapse.', 'Detached wheels and ground scrap semantics remain unresolved; source-visible shape takes precedence over this hypothesis.', 'Ground support is wheel-derived and remains provisional until exact receiver contact and full body checks.', 'No texture synthesis or approval.'])
    (DEST / 'fit.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__': main()
