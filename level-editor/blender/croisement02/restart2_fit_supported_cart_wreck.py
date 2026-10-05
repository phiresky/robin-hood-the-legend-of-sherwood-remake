"""Constrain a tipped wagon to simultaneous canopy and wheel ground support."""
import json, hashlib, math
import numpy as np
from PIL import Image, ImageDraw
from scipy.optimize import differential_evolution, brentq
import restart2_fit_south_cart_body as base


def main():
    prior_path = base.OUT / 'restart2-state/south-cart-body-fit-v2/fit.json'
    prior = json.loads(prior_path.read_text()); dest = base.OUT / 'restart2-state/south-cart-body-fit-v3'; dest.mkdir(exist_ok=False)
    native = Image.new('RGBA', (220, 180)); native.alpha_composite(Image.open(prior['source_frame']['image']).convert('RGBA'), (8, 24))
    def domain(polygon):
        im = Image.new('L', native.size); ImageDraw.Draw(im).polygon([(x + 8, y + 24) for x, y in polygon], fill=255)
        return (np.asarray(im) > 0) & (np.asarray(native)[:, :, 3] > 0)
    expected = domain(prior['body_domain_polygon']); canopy = domain(prior['canopy_domain_polygon'])
    def supported(parameters):
        def gap(angle):
            pieces, _ = base.geometry([angle, *parameters]); radians = math.radians(angle)
            wheel_minimum = min(-1.5 * math.sin(radians), 21 * abs(math.cos(radians)) - 3 * math.sin(radians))
            return min(v[2] for p in pieces for v in p['vertices']) - wheel_minimum
        angle = brentq(gap, 90, 115, xtol=1e-6)
        pieces, placement = base.geometry([angle, *parameters]); return pieces, placement, angle
    def objective(parameters):
        pieces, _, _ = supported(parameters); whole = base.silhouette(pieces); roof = base.silhouette([pieces[-1]])
        return int((expected & ~whole).sum()) * 2 + int((whole & ~expected).sum()) * 2 + int((roof & ~canopy).sum()) * 2
    result = differential_evolution(objective, [(22, 43), (38, 90), (8, 30), (2, 15), (8, 26)], seed=737, maxiter=100, popsize=10, polish=False)
    pieces, placement, angle = supported(result.x); mask = base.silhouette(pieces); roof = base.silhouette([pieces[-1]])
    diagnostic = np.asarray(native).copy(); diagnostic[expected & ~mask] = [255, 40, 200, 255]; diagnostic[mask & ~expected] = [40, 180, 255, 255]
    Image.fromarray(diagnostic).resize((880, 720), Image.Resampling.NEAREST).save(dest / 'native-fit.png')
    minimum = min(v[2] for p in pieces for v in p['vertices'])
    prior.update(status='Private source-constrained tipped wagon with analytical wheel/canopy common support; finite mesh reopening still required', prior_fit_sha256=hashlib.sha256(prior_path.read_bytes()).hexdigest(), parameters=[angle, *result.x.tolist()], placement=placement, missing_pixels=int((expected & ~mask).sum()), excess_pixels=int((mask & ~expected).sum()), roof_excess_pixels=int((roof & ~canopy).sum()), minimum_body_z=minimum, source_preserving_ground_shift=[0, minimum * base.COS / base.SIN, -minimum], pieces=pieces, support_scope='Analytical circle/finite canopy contacts share one plane; applied common camera-ray translation preserves native projection. Finite wheel mesh and load support must be reopened and verified.')
    (dest / 'fit.json').write_text(json.dumps(prior, indent=2) + '\n')


if __name__ == '__main__': main()
