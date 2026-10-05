"""Read-only alpha-aware attribution and retreat bounds for sign blockers."""
import json
import sys
from pathlib import Path
from collections import Counter

import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN, COS, RAY
from render_slots import acquire, release
from evidence_io import sha, write_json
from stage_review_scene import signature


class Surface:
    def __init__(self, objects):
        vertices, triangles, records = [], [], []
        self.images = {}
        self.components = {}
        for asset, obj in objects:
            mesh = obj.data
            mesh.calc_loop_triangles()
            world = np.array([tuple(obj.matrix_world @ v.co) for v in mesh.vertices])
            parent = list(range(len(world)))
            def find(a):
                while parent[a] != a:
                    parent[a] = parent[parent[a]]
                    a = parent[a]
                return a
            for polygon in mesh.polygons:
                root = find(polygon.vertices[0])
                for v in polygon.vertices[1:]:
                    parent[find(v)] = root
            ownership = mesh.color_attributes.get('Source ownership')
            groups = {}
            roles = {}
            for polygon in mesh.polygons:
                material = mesh.materials[polygon.material_index]
                observed = material.get('foliage_observed')
                red = [ownership.data[i].color[0] for i in polygon.loop_indices] if ownership else []
                role = ('observed' if observed is True or (red and max(red) > .5)
                        else 'inferred' if observed is False and red and max(red) < .5 else 'unclassified')
                roles[polygon.index] = role
                component = find(polygon.vertices[0])
                group = groups.setdefault(component, dict(vertices=set(), polygons=[], roles=Counter()))
                group['vertices'].update(polygon.vertices)
                group['polygons'].append(polygon.index)
                group['roles'][role] += 1
            for component, group in groups.items():
                points = world[sorted(group['vertices'])]
                self.components[(obj.name, component)] = dict(asset=asset, object=obj.name, component=component,
                    vertices=len(group['vertices']), faces=len(group['polygons']), roles=dict(group['roles']),
                    bounds_min=points.min(0).tolist(), bounds_max=points.max(0).tolist(),
                    inferred_only=set(group['roles']) == {'inferred'})
            offset = len(vertices)
            vertices.extend(world.tolist())
            uv = mesh.uv_layers.get('Foliage UV') or mesh.uv_layers.active
            for tri in mesh.loop_triangles:
                material = mesh.materials[tri.material_index]
                image_nodes = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
                foliage = bool(material.get('foliage_physical_opacity'))
                image = image_nodes[0].image if foliage else None
                if foliage and (len(image_nodes) != 1 or uv is None):
                    raise ValueError('Unsupported foliage shader/UV: ' + material.name)
                if image and image.name not in self.images:
                    w, h = image.size
                    self.images[image.name] = np.asarray(image.pixels[:], dtype=np.float32).reshape(h, w, 4)[:, :, 3].copy()
                points = world[list(tri.vertices)]
                normal = np.cross(points[1]-points[0], points[2]-points[0])
                length = np.linalg.norm(normal)
                if length <= 1e-12:
                    raise ValueError('Degenerate source triangle')
                normal /= length
                triangles.append(tuple(offset+i for i in tri.vertices))
                records.append(dict(asset=asset, object=obj.name, polygon=tri.polygon_index,
                    component=find(tri.vertices[0]), role=roles[tri.polygon_index],
                    material=material.name, one_sided=bool(material.node_tree.nodes.get('One-sided foliage')),
                    points=points, normal=normal, uv=np.array([tuple(uv.data[i].uv) for i in tri.loops]) if uv else None,
                    image=image.name if image else None, minimum_vertex_z=float(points[:, 2].min())))
        self.tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
        self.records = records

    def intersections(self, origin, maximum=10000):
        current = origin.copy()
        travelled = 0.
        for _ in range(1024):
            hit, normal, index, distance = self.tree.ray_cast(current, -RAY, maximum-travelled)
            if hit is None:
                return
            travelled += distance
            record = self.records[index]
            alpha = 1.
            if record['one_sided'] and float(record['normal'] @ np.asarray(RAY)) <= 0:
                alpha = 0.
            if alpha and record['image']:
                a, b, c = record['points']
                basis = np.column_stack((b-a, c-a))
                bc = np.linalg.lstsq(basis, np.asarray(hit)-a, rcond=None)[0]
                uv = np.array([1-bc.sum(), *bc]) @ record['uv']
                image = self.images[record['image']]
                h, w = image.shape
                x, y = np.floor(uv*[w, h]).astype(int)
                alpha = float(image[y % h, x % w])
            if alpha > .5:
                yield hit, travelled, record
            current = hit - RAY*.0005
            travelled += .0005
            if travelled >= maximum:
                return
        raise ValueError('Ray traversal exceeded explicit1024-surface budget')


def native_alpha(index, crop):
    order = json.loads((OUT / 'state-sign-candidate/native-order-reference-v3/manifest.json').read_text())
    animations = json.loads((OUT / 'animation-references/manifest.json').read_text())['animations']
    row = next(r for r in order['records'] if r['target_index'] == index)
    canvas = Image.new('RGBA', (1792, 1152))
    for overlay in row['overlapping_animations']:
        assert overlay['after_sign']
        f = next(a for a in animations if a['index'] == overlay['index'])['frames'][0]
        canvas.alpha_composite(Image.open(f['image']).convert('RGBA'), tuple(f['bbox'][:2]))
    return np.asarray(canvas.crop(crop))[:, :, 3] > 127


def main():
    dest = OUT / 'restart2-fence/sign-fragment-bounds-v1'
    dest.mkdir(exist_ok=False)
    proof = OUT / 'restart2-fence/sign-neighbors-v4'
    manifest = json.loads((proof / 'manifest.json').read_text())
    assembly_path = OUT / 'state-sign-candidate/five-instances-v3/assembly.json'
    assembly = json.loads(assembly_path.read_text())
    model = assembly_path.parent / 'model.blend'
    assert sha(model) == manifest['sign_model_sha256']
    results = []
    for target, keys in [(5, ['shrub-77']), (7, ['shrub-57']), (8, ['15', '16'])]:
        row = next(r for r in assembly['instances'] if r['target_index'] == target)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        objects = []
        for key in keys:
            source = manifest['inputs'][key]
            path = Path(source['worker']) / 'model.blend'
            assert sha(path) == source['model_sha256']
            with bpy.data.libraries.load(str(path), link=False) as (src, data):
                data.objects = list(source['objects'])
            for obj in data.objects:
                scene.collection.objects.link(obj)
                before = signature(obj)
                matrix = obj.matrix_world.copy()
                obj.parent = None
                obj.matrix_world = matrix
                assert before == signature(obj)
                objects.append((key, obj))
        surfaces = Surface(objects)
        x, y = row['native_target']['position_x'], row['native_target']['position_y']
        crop = (x-48, y-64, x+48, y+32)
        overlay = native_alpha(target, crop)
        pixels, fragments = [], {}
        for phase in [0, 8, 16, 24]:
            scene.frame_set(1+phase*2)
            bpy.context.view_layer.update()
            bodies = [('sign', scene.objects[n]) for n in row['parts']
                      if 'native_body_frame' in scene.objects[n] and scene.objects[n].scale.x > .5]
            sign = Surface(bodies)
            alone = np.asarray(Image.open(proof / f'target-{target}/pose-{phase:02}-body-alone.png').convert('RGB'))[1::3,1::3,0] > 127
            joint = np.asarray(Image.open(proof / f'target-{target}/pose-{phase:02}-body-first-hit.png').convert('RGB'))[1::3,1::3,0] > 127
            yy, xx = np.nonzero(alone & ~joint & ~overlay)
            for py, px in zip(yy, xx):
                sx, sy = crop[0]+int(px)+.5, crop[1]+int(py)+.5
                origin = Vector((sx, -sy/SIN, 0)) + RAY*5000
                body_hits = list(sign.intersections(origin))
                if not body_hits:
                    pixels.append(dict(phase=phase, source_pixel=[sx-.5, sy-.5], status='Raster/ray contour discrepancy'))
                    continue
                front, back = body_hits[0][1], body_hits[-1][1]
                hits = list(surfaces.intersections(origin, maximum=front-.001))
                stack = []
                for hit, distance, record in hits:
                    key = (record['asset'], record['object'], record['polygon'])
                    retreat = back-distance+.1
                    component = surfaces.components[(record['object'], record['component'])]
                    fragment = fragments.setdefault(key, dict(asset=record['asset'], object=record['object'],
                        polygon=record['polygon'], component=record['component'], role=record['role'],
                        material=record['material'], component_inferred_only=component['inferred_only'],
                        component_roles=component['roles'], component_vertices=component['vertices'],
                        component_minimum_z=component['bounds_min'][2], maximum_retreat=0., samples=0))
                    fragment['maximum_retreat'] = max(fragment['maximum_retreat'], retreat)
                    fragment['samples'] += 1
                    fragment['component_minimum_z_after_retreat'] = fragment['component_minimum_z'] - SIN*fragment['maximum_retreat']
                    stack.append(dict(asset=record['asset'], object=record['object'], polygon=record['polygon'],
                                      component=record['component'], role=record['role'],
                                      component_inferred_only=component['inferred_only'], required_retreat=retreat,
                                      hit_z=float(hit.z)))
                pixels.append(dict(phase=phase, source_pixel=[sx-.5, sy-.5], status='Attributed' if stack else 'No alpha-aware blocker on exact center ray', blockers=stack))
            print('TARGET_PHASE', target, phase, 'pixels', len(xx), flush=True)
        attributed = [p for p in pixels if p.get('blockers')]
        constrained = [p for p in attributed if any(not b['component_inferred_only'] for b in p['blockers'])]
        result = dict(target_index=target, inputs={k:manifest['inputs'][k] for k in keys},
            raster_excess_samples=len(pixels), alpha_aware_attributed_samples=len(attributed),
            samples_with_non_inferred_blocker=len(constrained),
            samples_with_only_inferred_blockers=len(attributed)-len(constrained),
            first_hit_roles=dict(Counter(p['blockers'][0]['role'] for p in attributed)),
            fragments=list(fragments.values()), pixels=pixels)
        write_json(dest / f'target-{target}.json', result)
        results.append({k:v for k,v in result.items() if k not in ['fragments','pixels']})
    assert sha(model) == manifest['sign_model_sha256']
    assert all(sha(Path(r['worker']) / 'model.blend') == r['model_sha256'] for r in manifest['inputs'].values())
    write_json(dest / 'report.json', dict(status='Read-only fragment attribution and source-ray retreat bounds',
        source_manifest_sha256=sha(proof / 'manifest.json'), sign_model_sha256=sha(model), results=results,
        direction=list(-RAY), semantics='Retreat behind the complete intersected sign body plus0.1unit. Source screen coordinates stay algebraically unchanged; no geometry has moved.',
        limitations=['Four sign poses and native canopy frame0 only.',
                     'Observed or mixed-ownership connected components are ineligible for inferred-only corrections.',
                     'Computed ground minimum uses worldZ0; exact bank/tree support is an additional stricter constraint.',
                     'A geometrically possible retreat is not proof of coherent volume or unchanged full-source RGB; those require fresh physical review.']))
    print(dest)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
