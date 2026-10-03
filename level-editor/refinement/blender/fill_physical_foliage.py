"""Fill reviewed unknown foliage RGB while retaining its physical cutout atlas."""
import numpy as np


def triangle_pixels(uv, width, height, offset=(0., 0.)):
    points = np.asarray(uv) * [width, height] - offset
    low = np.maximum(0, np.ceil(points.min(0) - .5).astype(int))
    high = np.minimum([width - 1, height - 1], np.floor(points.max(0) - .5).astype(int))
    if np.any(high < low):
        return np.empty(0, int), np.empty(0, int), np.empty((0, 3))
    x, y = np.meshgrid(np.arange(low[0], high[0] + 1), np.arange(low[1], high[1] + 1))
    matrix = np.stack([points[1] - points[0], points[2] - points[0]], axis=1)
    if abs(np.linalg.det(matrix)) < 1e-12:
        raise ValueError('Degenerate physical foliage UV triangle')
    weights = (np.stack([x.ravel() + .5, y.ravel() + .5], axis=1) - points[0]) @ np.linalg.inv(matrix).T
    barycentric = np.column_stack([1 - weights.sum(1), weights])
    inside = (barycentric >= -1e-7).all(1)
    return y.ravel()[inside], x.ravel()[inside], barycentric[inside]


def fill_atlas_edges(colors, generated, eligible, radius):
    """Extrapolate only from original generated texels in this unknown atlas."""
    if type(radius) is not int or radius not in (0, 1, 2):
        raise ValueError('Foliage edge fill radius must be 0, 1, or 2')
    result = colors.copy()
    repaired = np.zeros(generated.shape, bool)
    height, width = generated.shape
    offsets = sorted((x*x+y*y, y, x) for y in range(-radius, radius+1)
                     for x in range(-radius, radius+1) if 0 < x*x+y*y <= radius*radius)
    for _, dy, dx in offsets:
        rows, cols = np.where(eligible & ~generated & ~repaired)
        sy, sx = rows+dy, cols+dx
        inside = (sy >= 0) & (sy < height) & (sx >= 0) & (sx < width)
        rows, cols, sy, sx = rows[inside], cols[inside], sy[inside], sx[inside]
        take = generated[sy, sx]
        result[rows[take], cols[take], :3] = colors[sy[take], sx[take], :3]
        repaired[rows[take], cols[take]] = True
    if repaired.sum() > eligible.sum() * .2:
        raise ValueError('Foliage edge fill exceeds 20% of the physical atlas')
    return result, repaired


def fill(objects, sample, face_scope, generated_hash, *, subpixels=False, sample_grid=0, edge_fill_radius=0):
    if type(subpixels) is not bool:
        raise ValueError('Foliage subpixel sampling must be an explicit boolean')
    if type(sample_grid) is not int or sample_grid not in (0, 4, 8):
        raise ValueError("Foliage sample grid must be 0, 4, or 8")
    radii = edge_fill_radius if isinstance(edge_fill_radius, dict) else None
    values = radii.values() if radii is not None else [edge_fill_radius]
    if any(type(value) is not int or value not in (0, 1, 2) for value in values):
        raise ValueError('Foliage edge fill radius must be 0, 1, or 2')
    if radii is not None:
        names = {m.name for obj in objects for m in obj.data.materials if m and m.get('foliage_physical_opacity')}
        if not radii or not set(radii) <= names:
            raise ValueError('Foliage edge fill names absent or non-foliage materials')
    reports = []
    for obj in objects:
        mesh = obj.data
        mesh.calc_loop_triangles()
        ownership = mesh.color_attributes.get('Source ownership')
        for slot, material in enumerate(list(mesh.materials)):
            if not material or not material.get('foliage_physical_opacity'):
                continue
            if material.get('source_ownership_channel') != 'vertex-color-r' or ownership is None or ownership.domain != 'CORNER':
                raise ValueError('Physical foliage requires explicit corner source ownership')
            faces = [f for f in mesh.polygons if f.material_index == slot]
            flags = {f.index: [ownership.data[i].color[0] for i in f.loop_indices] for f in faces}
            if any(any(v not in (0., 1.) for v in values) or len(set(values)) != 1 for values in flags.values()):
                raise ValueError('Mixed source ownership on a foliage face is unsupported')
            unknown = {f.index for f in faces if flags[f.index][0] == 0 and
                       (face_scope is None or f.index in face_scope.get(obj.name, []))}
            if not unknown:
                continue
            # Each cutout material owns one atlas. Reject overlapping known/unknown
            # use rather than allowing an atlas write to alter a protected face.
            if len(unknown) != len(faces):
                raise ValueError('Physical foliage atlas mixes protected and editable faces')
            textures = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
            if len(textures) != 1:
                raise ValueError('Expected one physical foliage texture')
            texture = textures[0]
            links = texture.inputs['Vector'].links
            if len(links) != 1 or links[0].from_node.type != 'UVMAP':
                raise ValueError('Expected explicit foliage UV map')
            uv = mesh.uv_layers[links[0].from_node.uv_map]
            width, height = texture.image.size
            before = np.empty(width * height * 4, dtype=np.float32)
            texture.image.pixels.foreach_get(before)
            before = before.reshape(height, width, 4)
            after = before.copy()
            written = np.zeros((height, width), bool)
            visible = np.zeros_like(written)
            subpixel_generated = 0
            grid_generated = 0
            for triangle in mesh.loop_triangles:
                if triangle.polygon_index not in unknown:
                    continue
                rows, cols, weights = triangle_pixels([uv.data[i].uv[:] for i in triangle.loops], width, height)
                keep = (before[rows, cols, 3] >= .5) & ~written[rows, cols]
                rows, cols, weights = rows[keep], cols[keep], weights[keep]
                if not len(rows):
                    continue
                visible[rows, cols] = True
                points = np.asarray([obj.matrix_world @ mesh.vertices[i].co for i in triangle.vertices])
                positions = weights @ points
                normal = (obj.matrix_world.to_3x3().inverted().transposed() @ triangle.normal).normalized()
                colors = before[rows, cols].copy()
                accepted = sample(obj, normal, positions, np.zeros(len(rows), bool), colors,
                                  face_index=triangle.polygon_index)
                after[rows[accepted], cols[accepted], :3] = colors[accepted, :3]
                written[rows[accepted], cols[accepted]] = True
                if subpixels:
                    triangle_uv = np.asarray([uv.data[i].uv[:] for i in triangle.loops]) * [width, height]
                    inverse = np.linalg.inv(np.stack([triangle_uv[1]-triangle_uv[0], triangle_uv[2]-triangle_uv[0]], axis=1))
                    for offset in [(-.25, -.25), (.25, -.25), (-.25, .25), (.25, .25)]:
                        delta = inverse @ np.asarray(offset)
                        shifted = weights + [-delta.sum(), *delta]
                        pending = ~written[rows, cols] & (shifted >= -1e-7).all(1)
                        indices = np.flatnonzero(pending)
                        if not len(indices):
                            continue
                        retry_colors = before[rows[indices], cols[indices]].copy()
                        filled = sample(obj, normal, shifted[indices] @ points,
                                        np.zeros(len(indices), bool), retry_colors,
                                        face_index=triangle.polygon_index, record_statistics=False)
                        selected = indices[filled]
                        after[rows[selected], cols[selected], :3] = retry_colors[filled, :3]
                        written[rows[selected], cols[selected]] = True
                        subpixel_generated += int(filled.sum())
            # Sample real surface points within each atlas texel, including
            # thin triangles that contain no texel centre. All visibility and
            # source ownership checks still run through the sampling callback.
            if sample_grid:
                for triangle in mesh.loop_triangles:
                    if triangle.polygon_index not in unknown:
                        continue
                    triangle_uv = [uv.data[i].uv[:] for i in triangle.loops]
                    points = np.asarray([obj.matrix_world @ mesh.vertices[i].co for i in triangle.vertices])
                    normal = (obj.matrix_world.to_3x3().inverted().transposed() @ triangle.normal).normalized()
                    for y in range(sample_grid):
                        for x in range(sample_grid):
                            offset = ((x + .5) / sample_grid - .5, (y + .5) / sample_grid - .5)
                            rows, cols, weights = triangle_pixels(triangle_uv, width, height, offset)
                            keep = (before[rows, cols, 3] >= .5) & ~written[rows, cols]
                            rows, cols, weights = rows[keep], cols[keep], weights[keep]
                            if not len(rows):
                                continue
                            visible[rows, cols] = True
                            colors = before[rows, cols].copy()
                            accepted = sample(obj, normal, weights @ points, np.zeros(len(rows), bool), colors,
                                              face_index=triangle.polygon_index, record_statistics=False)
                            after[rows[accepted], cols[accepted], :3] = colors[accepted, :3]
                            written[rows[accepted], cols[accepted]] = True
                            grid_generated += int(accepted.sum())
            extrapolated = np.zeros_like(written)
            radius = radii.get(material.name, 0) if radii is not None else edge_fill_radius
            if radius:
                after, extrapolated = fill_atlas_edges(after, written, visible & (before[..., 3] >= .5), radius)
                written |= extrapolated
            if not np.array_equal(before[..., 3], after[..., 3]) or not np.array_equal(before[~written], after[~written]):
                raise ValueError('Foliage fill changed protected RGB or physical alpha')
            if written.any():
                replacement = material.copy()
                image = texture.image.copy()
                image.pixels.foreach_set(after.ravel())
                image.pack()
                replacement.node_tree.nodes[texture.name].image = image
                replacement['generated_source_sha256'] = generated_hash
                replacement['texture_review_status'] = 'candidate-review-pending'
                mesh.materials[slot] = replacement
                actual = np.empty(after.size, np.float32)
                image.pixels.foreach_get(actual)
                actual = actual.reshape(after.shape)
                if not np.array_equal(actual[..., 3], before[..., 3]) or not np.array_equal(actual[~written], before[~written]):
                    raise ValueError('Stored foliage texture changed protected RGB or alpha')
                if np.max(np.abs(actual[written, :3] - after[written, :3])) > 1 / 255 + 1e-7:
                    raise ValueError('Stored foliage RGB exceeds byte quantization error')
            reports.append(dict(object=obj.name, material=material.name, generated=int((written & ~extrapolated).sum()),
                                extrapolated=int(extrapolated.sum()), edge_fill_radius=radius,
                                unfilled=int((visible & ~written).sum()), physical_alpha_changed=0,
                                protected_rgb_changed=0, subpixel_generated=subpixel_generated, grid_generated=grid_generated))
    if radii is not None and set(radii) - {row['material'] for row in reports}:
        raise ValueError('Foliage edge fill includes protected or excluded materials')
    return reports
