"""Irregular interior crown volume retaining its complete native source domain."""
import math
from pathlib import Path
import numpy as np
from PIL import Image
from mathutils import Vector
from tree_geometry import SIN, COS, RAY, material, one_sided, replace_mesh


def build(obj, packet, ground_y):
    path = Path(packet['lobes'][0]['image']).parent / 'complete-source.png'
    source = np.asarray(Image.open(path).convert('RGBA'))
    alpha = source[:, :, 3] > 127
    x0, y0, width, height = packet['native_bbox']
    fx, fy, fw, fh = packet['bbox']
    image_center_y = fy + fh / 2
    boundary = int(fx + fw / 2)
    radius_x = fw * .5 + 12
    radii = np.array([radius_x, radius_x * 1.22, max(fh * .56, radius_x * .70)])
    center = np.array([float(boundary), -ground_y / SIN, (ground_y - image_center_y) / COS])
    if center[2] - radii[2] < 50:
        center += np.asarray(RAY) * ((50 + radii[2] - center[2]) / SIN)
    mats = [material(obj.name + ' observed patches', path, True),
            material(obj.name + ' inferred leaf volume', path, False),
            material(obj.name + ' inferred leaf backs', path, False)]
    one_sided(mats[0])
    one_sided(mats[2])
    vertices, faces, uvs, slots, known = [], [], [], [], []
    rng = np.random.default_rng(24040 + boundary)

    def quad(points, coords, observed=False, inferred_slot=1):
        start = len(vertices)
        vertices.extend([list(p) for p in points])
        uvs.extend(coords)
        faces.extend([(start + 2, start + 1, start), (start + 3, start + 2, start)] if observed
                     else [(start, start + 1, start + 2), (start, start + 2, start + 3)])
        slots.extend([0 if observed else inferred_slot] * 2)
        known.extend([observed] * 2)
        if observed:
            rear = len(vertices)
            vertices.extend([list(np.asarray(p) - np.asarray(RAY) * .02) for p in points])
            uvs.extend(coords)
            faces.extend([(rear, rear + 1, rear + 2), (rear, rear + 2, rear + 3)])
            slots.extend([2, 2])
            known.extend([False, False])

    def point(x, y, depth):
        return center + np.array([x - boundary, -(y - image_center_y) * SIN,
                                  -(y - image_center_y) * COS]) + np.asarray(RAY) * depth

    ray = np.asarray(RAY)
    a = np.sum((ray / radii) ** 2)
    count = 0
    for row_top in range(0, height, 12):
        for left in range(0, width, 12):
            left = max(0, left + int(rng.integers(-3, 4)))
            top_sample = max(0, row_top + int(rng.integers(-3, 4)))
            right, bottom = min(width, left + 22), min(height, top_sample + 22)
            top = top_sample
            if not alpha[top:bottom, left:right].any():
                continue
            xa, xb, ya, yb = x0 + left, x0 + right, y0 + top, y0 + bottom
            relative = point((xa + xb) / 2, (ya + yb) / 2, 0) - center
            b = 2 * np.sum(relative * ray / radii ** 2)
            c = np.sum((relative / radii) ** 2) - 1
            discriminant = max(0., b * b - 4 * a * c)
            low, high = (-b - math.sqrt(discriminant)) / (2 * a), (-b + math.sqrt(discriminant)) / (2 * a)
            uv = [(left / width, 1 - top / height), (right / width, 1 - top / height),
                  (right / width, 1 - bottom / height), (left / width, 1 - bottom / height)]
            for fraction in [.12 + rng.random() * .12, .44 + rng.random() * .12, .76 + rng.random() * .12]:
                depth = low + fraction * (high - low)
                quad([point(xa, ya, depth), point(xb, ya, depth), point(xb, yb, depth), point(xa, yb, depth)], uv, True)
                count += 1
    patches = [(x, y) for y in range(0, height - 24, 8) for x in range(0, width - 24, 8)
               if alpha[y:y + 24, x:x + 24].mean() > .55]
    if not patches:
        raise ValueError('No leafy source patches for inferred volume')
    atlas = np.zeros((80 * 24, 64 * 24, 4), dtype=np.uint8)
    atlas_index = 0
    sample_v, sample_u = np.mgrid[0:24, 0:24] / 24 + .5 / 24
    for _ in range(600):
        unit = rng.normal(size=3)
        unit /= np.linalg.norm(unit)
        unit *= rng.uniform(.05, 1.) ** (1 / 3)
        position = center + unit * radii
        # The inferred volume follows the observed crown height and narrows
        # toward its outer silhouette.
        projected_y = -position[1] * SIN - position[2] * COS
        if ((position[0] - boundary) / radii[0]) ** 2 + ((projected_y - image_center_y) / (fh * .52)) ** 2 > 1:
            continue
        px, py = patches[int(rng.integers(len(patches)))]
        uv = [(px / width, 1 - py / height), ((px + 24) / width, 1 - py / height),
              ((px + 24) / width, 1 - (py + 24) / height), (px / width, 1 - (py + 24) / height)]
        axis = Vector(rng.normal(size=3)).normalized()
        other = axis.cross(Vector((0, 0, 1)) if abs(axis.z) < .9 else Vector((1, 0, 0))).normalized()
        third = axis.cross(other).normalized()
        size = rng.uniform(9., 15.)
        for u, v in [(axis, other), (axis, third), (other, third)]:
            points = [position + (np.asarray(u) * su + np.asarray(v) * sv) * size
                      for su, sv in [(-1, -1), (1, -1), (1, 1), (-1, 1)]]
            if np.dot(np.cross(points[1]-points[0], points[2]-points[0]), ray) > 0:
                points.reverse()
            # Native alpha gates the inferred volume in every renderer.
            p0, p1, p2, p3 = [np.asarray(p) for p in points]
            sample = ((1 - sample_u)[..., None] * (1 - sample_v)[..., None] * p0
                      + sample_u[..., None] * (1 - sample_v)[..., None] * p1
                      + sample_u[..., None] * sample_v[..., None] * p2
                      + (1 - sample_u)[..., None] * sample_v[..., None] * p3)
            source_x = sample[..., 0]
            ix = np.floor(source_x - x0).astype(int)
            iy = np.floor(-sample[..., 1] * SIN - sample[..., 2] * COS - y0).astype(int)
            valid = (ix >= 0) & (ix < width) & (iy >= 0) & (iy < height)
            edge_alpha = np.zeros((24, 24))
            edge_alpha[valid] = alpha[iy[valid], ix[valid]]
            tile = source[py:py + 24, px:px + 24].copy()
            # Fill hidden depth with the same irregular volume.
            # Its source-facing projection stays inside the native domain;
            # these hidden leaf placements remain explicitly inferred.
            tile[..., 3] = np.rint(tile[..., 3] * edge_alpha).astype(np.uint8)
            ax, ay = atlas_index % 64 * 24, atlas_index // 64 * 24
            atlas[ay:ay + 24, ax:ax + 24] = tile
            atlas_uv = [(ax / atlas.shape[1], 1 - ay / atlas.shape[0]),
                        ((ax + 24) / atlas.shape[1], 1 - ay / atlas.shape[0]),
                        ((ax + 24) / atlas.shape[1], 1 - (ay + 24) / atlas.shape[0]),
                        (ax / atlas.shape[1], 1 - (ay + 24) / atlas.shape[0])]
            quad(points, atlas_uv, inferred_slot=4)
            atlas_index += 1
    atlas_path = path.parent / 'inferred-interior-atlas.png'
    Image.fromarray(atlas).save(atlas_path)
    mats.append(material(obj.name + ' inferred outer leaf tiles', atlas_path, False))
    mats.append(material(obj.name + ' inferred inner leaf backs', atlas_path, False))
    one_sided(mats[4])
    result = replace_mesh(obj, vertices, faces, uvs, mats, slots, known)
    points = np.asarray(vertices)
    result.update(geometry_version='native-leaf-clusters-v6', width=float(np.ptp(points[:, 0])),
        depth=float(np.ptp(points[:, 1])), source_projection_preserved=True, leaf_clusters=count + atlas_index // 3,
        inferred_off_map_half=False, method='World-space ellipsoid; jittered observed patches and randomly rotated inferred rear leaf clusters',
        tree_references=['leicester-southeast-cottage-tree', 'leicester-moat-bank-tree'])
    return result
