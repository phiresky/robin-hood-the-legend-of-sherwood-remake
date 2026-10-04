"""Conservative continuous-projection occlusion, independent of sampled hit grids."""
import numpy as np
from shapely import affinity, union_all
from shapely.geometry import Polygon, box
from shapely.strtree import STRtree


def opaque_footprint(uv, alpha, projected, extension='REPEAT'):
    """Exact nearest-filter alpha texel rectangles intersected with a UV triangle."""
    height, width = alpha.shape
    pixels = np.asarray(uv, dtype=float) * [width, height]
    triangle = Polygon(pixels)
    if not triangle.is_valid or triangle.area < 1e-12:
        raise ValueError('Degenerate physical UV triangle')
    low = np.floor(pixels.min(axis=0)).astype(int)
    high = np.ceil(pixels.max(axis=0)).astype(int)
    if np.prod(high - low) > 262144:
        raise ValueError('Unbounded per-triangle opacity footprint')
    xs = np.arange(low[0], high[0])
    patches = []
    for y in range(low[1], high[1]):
        if extension == 'REPEAT':
            row = alpha[y % height, xs % width]
        elif extension == 'EXTEND':
            row = alpha[min(height - 1, max(0, y)), np.clip(xs, 0, width - 1)]
        elif extension == 'CLIP':
            row = np.zeros(len(xs), bool)
            inside = (xs >= 0) & (xs < width)
            if 0 <= y < height:
                row[inside] = alpha[y, xs[inside]]
        else:
            raise ValueError('Unsupported physical texture extension')
        changes = np.flatnonzero(np.diff(np.r_[False, row, False]))
        for start, end in zip(changes[::2], changes[1::2]):
            part = box(low[0] + start, y, low[0] + end, y + 1).intersection(triangle)
            if not part.is_empty:
                patches.append(part)
    if not patches:
        return Polygon()
    transform = np.linalg.solve(np.c_[pixels, np.ones(3)], np.asarray(projected))
    return affinity.affine_transform(union_all(patches), [transform[0, 0], transform[1, 0],
        transform[0, 1], transform[1, 1], transform[2, 0], transform[2, 1]])


class ObservedOcclusion:
    def __init__(self, footprints, depth_minima, *, projection_margin=.002, depth_margin=.05):
        if projection_margin <= 0 or depth_margin <= 0 or len(footprints) != len(depth_minima):
            raise ValueError('Occlusion proof requires positive safety margins and aligned inputs')
        self.margin = projection_margin
        self.depth_margin = depth_margin
        self.polygons = [p.buffer(-projection_margin) for p in footprints]
        self.depths = np.asarray(depth_minima)
        self.tree = STRtree(self.polygons)

    def prove(self, target, depth_maximum):
        # Expanding the target and shrinking occluders makes boundary uncertainty
        # retain source protection rather than invent a hidden region.
        target = target.buffer(self.margin)
        indices = [int(i) for i in self.tree.query(target)
                   if self.depths[i] > depth_maximum + self.depth_margin]
        if target.is_empty:
            return dict(hidden=True, opaque_projected_area=0., uncovered_area=0., occluders=[])
        if not indices:
            return dict(hidden=False, opaque_projected_area=float(target.area), uncovered_area=float(target.area), occluders=[])
        covering = union_all([self.polygons[i] for i in indices])
        remainder = target.difference(covering)
        return dict(hidden=bool(remainder.is_empty), opaque_projected_area=float(target.area),
                    uncovered_area=float(remainder.area), occluders=indices)
