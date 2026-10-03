"""Fill reviewed unknown foliage RGB while retaining its physical cutout atlas."""
import numpy as np


def triangle_pixels(uv, width, height):
    points = np.asarray(uv) * [width, height]
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


def fill(objects, sample, face_scope, generated_hash):
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
            reports.append(dict(object=obj.name, material=material.name, generated=int(written.sum()),
                                unfilled=int((visible & ~written).sum()), physical_alpha_changed=0,
                                protected_rgb_changed=0))
    return reports
