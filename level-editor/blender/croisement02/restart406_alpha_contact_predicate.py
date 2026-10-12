"""Threshold-only bilinear contact predicate with conservative early decisions."""
import math

import numpy as np

from restart14_butterfly07_prism_contacts import alpha_maximum, wrapped


def sample_alpha(point, image, sampler, factor):
    height, width = image.shape[:2]
    x, y = point[3:5] * [width, height] - .5
    ix, iy = math.floor(x), math.floor(y)
    u, v = x - ix, y - iy
    values = [image[wrapped(iy + dy, height, sampler.get('wrapT', 10497)),
                    wrapped(ix + dx, width, sampler.get('wrapS', 10497)), 3] / 255 * factor
              for dx, dy in [(0, 0), (1, 0), (0, 1), (1, 1)]]
    return ((1-u)*(1-v)*values[0] + u*(1-v)*values[1]
            + (1-u)*v*values[2] + u*v*values[3])


def threshold_contact(poly, image, sampler, factor, cutoff):
    """Return a verified witness or conservative exclusion, never a guessed maximum."""
    assert len(poly) and 0 <= factor <= 1 and 0 < cutoff <= 1
    if image is None:
        return dict(intersects=factor >= cutoff, value=factor,
                    witness=poly.mean(0) if factor >= cutoff else None,
                    mode='constant', cells=0)
    height, width = image.shape[:2]
    uv = poly[:, 3:5] * [width, height] - .5
    low = np.floor(uv.min(0)).astype(int)
    high = np.floor(uv.max(0)).astype(int)
    if np.prod(high-low+1) > 100000:
        raise ValueError('Unexpectedly large UV contact; partition explicitly before proceeding')
    # A bilinear interpolant is a convex combination of its four cell corners.
    xs = [wrapped(x, width, sampler.get('wrapS', 10497))
          for x in range(low[0], high[0]+2)]
    ys = [wrapped(y, height, sampler.get('wrapT', 10497))
          for y in range(low[1], high[1]+2)]
    upper = float(image[np.asarray(ys)[:, None], np.asarray(xs)[None, :], 3].max()) / 255 * factor
    if upper < cutoff:
        return dict(intersects=False, upper_bound=upper, witness=None,
                    mode='rectangle-upper-bound', cells=0)
    # Polygon vertices and its centroid belong to this convex clipped polygon.
    for point in [*poly, poly.mean(0)]:
        value = float(sample_alpha(point, image, sampler, factor))
        if value >= cutoff:
            return dict(intersects=True, value=value, witness=point.copy(),
                        mode='verified-point', cells=0)
    value, witness, cells = alpha_maximum(poly, image, sampler, factor)
    return dict(intersects=value >= cutoff, value=value,
                witness=witness if value >= cutoff else None,
                mode='exact-cell-maximum', cells=cells)
