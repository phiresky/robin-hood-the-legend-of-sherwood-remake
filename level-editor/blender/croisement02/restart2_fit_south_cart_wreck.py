"""Fit canopy and broken bed source domains separately with ground guards."""
import hashlib, json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.optimize import differential_evolution
import restart2_fit_south_cart_body as base


def main():
    dest = base.OUT / 'restart2-state/south-cart-body-fit-v2'; dest.mkdir(exist_ok=False)
    prior = json.loads((base.DEST / 'fit.json').read_text())
    frame = prior['source_frame']; image = Image.open(frame['image']).convert('RGBA')
    native = Image.new('RGBA', (220, 180)); native.alpha_composite(image, (8, 24))
    canopy_polygon = prior['body_domain_polygon']
    body_polygon = [(54, 64), (56, 0), (78, 2), (96, 14), (129, 19), (146, 42), (171, 35), (188, 44), (187, 62), (170, 91), (157, 114), (107, 120), (65, 99)]
    def domain(poly):
        im = Image.new('L', native.size); ImageDraw.Draw(im).polygon([(x + 8, y + 24) for x, y in poly], fill=255)
        return (np.asarray(im) > 0) & (np.asarray(native)[:, :, 3] > 0)
    expected, canopy = domain(body_polygon), domain(canopy_polygon)
    def objective(params):
        pieces, _ = base.geometry(params); minimum = min(v[2] for p in pieces for v in p['vertices'])
        if minimum < 0: return 50000 + abs(minimum) * 1000
        all_mask = base.silhouette(pieces); roof = base.silhouette([pieces[-1]])
        return int((expected & ~all_mask).sum()) * 2 + int((all_mask & ~expected).sum()) * 2 + int((roof & ~canopy).sum()) * 2
    result = differential_evolution(objective, [(60, 110), (22, 43), (38, 90), (8, 30), (2, 15), (8, 26)], seed=736, maxiter=150, popsize=12, polish=False)
    pieces, placement = base.geometry(result.x); mask = base.silhouette(pieces); roof = base.silhouette([pieces[-1]])
    diagnostic = np.asarray(native).copy(); diagnostic[expected & ~mask] = [255, 40, 200, 255]; diagnostic[mask & ~expected] = [40, 180, 255, 255]
    Image.fromarray(diagnostic).resize((880, 720), Image.Resampling.NEAREST).save(dest / 'native-fit.png')
    report = dict(status='Private whole wreck hypothesis; source gaps and hidden support require visual review', prior_fit_sha256=hashlib.sha256((base.DEST / 'fit.json').read_bytes()).hexdigest(), source_frame=frame, body_domain_polygon=body_polygon, canopy_domain_polygon=canopy_polygon, parameters=result.x.tolist(), placement=placement, expected_pixels=int(expected.sum()), missing_pixels=int((expected & ~mask).sum()), excess_pixels=int((mask & ~expected).sum()), roof_excess_pixels=int((roof & ~canopy).sum()), minimum_body_z=min(v[2] for p in pieces for v in p['vertices']), pieces=pieces, limitations=prior['limits'])
    (dest / 'fit.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__': main()
