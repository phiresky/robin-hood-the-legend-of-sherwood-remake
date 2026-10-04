"""Screen inferred net cord extensions against the current physical tree cutouts."""
import json
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT, tree_workspace
from tree_geometry import RAY, SIN
from log_trap_state_candidate import sha
from physical_opacity import OpacityRegistry
from render_slots import acquire, release


def main():
    acquire()
    try:
        base = OUT/'net-endpoint-candidate-v3'
        audit = json.loads((base/'tree-attachment-audit.json').read_text())
        bpy.ops.wm.read_factory_settings(use_empty=True)
        scene = bpy.context.scene
        objects, bindings = [], []
        for index in [43, 45, 46]:
            worker = tree_workspace(index)
            names = json.loads((worker/'modified/views.json').read_text())['object_names']
            with bpy.data.libraries.load(str(worker/'model.blend'), link=False) as (src, dst):
                dst.objects = names
            for obj in dst.objects:
                cursor = obj
                while cursor:
                    if cursor.name not in scene.objects:
                        scene.collection.objects.link(cursor)
                    cursor = cursor.parent
                objects.append(obj)
            bindings.append(dict(tree=index, model_sha256=sha(worker/'model.blend')))
        bpy.context.view_layer.update()
        vertices, triangles = [], []
        registry = OpacityRegistry()
        for obj in objects:
            mesh = obj.data
            mesh.calc_loop_triangles()
            offset = len(vertices)
            vertices.extend(obj.matrix_world @ v.co for v in mesh.vertices)
            for triangle in mesh.loop_triangles:
                triangles.append(tuple(offset+i for i in triangle.vertices))
                registry.add(obj, mesh, triangle)
        tree = registry.wrap(BVHTree.FromPolygons(vertices, triangles, all_triangles=True))
        rows = []
        for cord in audit['cords']:
            screened = []
            for proposal in cord['proposals']:
                start = Vector(cord['current_upper_world']) + RAY*proposal['camera_ray_shift']
                end = Vector(proposal['attachment_world'])
                samples = max(2, int((end-start).length*2))
                exposed = []
                for i in range(samples):
                    point = start.lerp(end, i/(samples-1))
                    hit = tree.ray_cast(point+RAY*3000, -RAY, 2999.99)
                    if hit[0] is None:
                        exposed.append(dict(sample=i, world=list(point)))
                screened.append(dict(proposal=proposal, samples=samples, exposed=len(exposed), exposed_points=exposed))
            screened.sort(key=lambda r: (r['exposed']/r['samples'], abs(r['proposal']['camera_ray_shift'])))
            rows.append(dict(cord=cord['cord'], hypotheses=screened))
        # A hidden attachment can still place the body behind the wrong foliage.
        from PIL import Image
        import numpy as np
        model = json.loads((base/'manifest.json').read_text())
        with bpy.data.libraries.load(str(base/'worker.blend'), link=False) as (src, dst):
            dst.objects = [r['object'] for r in model['objects']]
        net_objects = dst.objects
        for obj in net_objects:
            scene.collection.objects.link(obj)
        bpy.context.view_layer.update()
        fit = json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text())
        source = next(r for r in fit['records'] if r['family']=='piege01' and r['variant']=='i')
        x, y, w, h = source['bbox']
        alpha = np.asarray(Image.open(source['source']).convert('RGBA'))[:, :, 3] > 127
        tests = []
        for bag_shift, wood_shift in [(0,0), (54,87), (66,87), (-32,87), (-38,87)]:
            points, faces = [], []
            for obj in net_objects:
                shift = bag_shift if obj.name in ('Occupied bag','Bag hanging cord') else wood_shift
                offset = len(points)
                points.extend(obj.matrix_world @ v.co + RAY*shift for v in obj.data.vertices)
                faces.extend(tuple(offset+i for i in p.vertices) for p in obj.data.polygons)
            body = BVHTree.FromPolygons(points, faces)
            received, hidden = 0, 0
            for iy, ix in np.argwhere(alpha):
                origin = Vector((x+int(ix)+.5, -(y+int(iy)+.5)/SIN, 0)) + RAY*3000
                hit = body.ray_cast(origin, -RAY, 6000)
                if hit[0] is not None:
                    received += 1
                    cover = tree.ray_cast(origin, -RAY, max(0,hit[3]-.01))
                    hidden += cover[0] is not None
            interval = model['depth_inference']['remaining_interval']
            relative_shift = wood_shift-bag_shift
            intersects = interval['first'] < relative_shift < interval['last']
            tests.append(dict(bag_shift=bag_shift, wood_shift=wood_shift, native_pixels=int(alpha.sum()), body_received=received, hidden_by_physical_trees=hidden, convex_bag_wood_overlap=intersects, overlap_basis='Reopened v3 convex SAT interval translated by the relative camera-ray shifts'))
        result = dict(status='Private attachment screening; no tree or net model changed', tree_bindings=bindings, attachment_audit_sha256=sha(base/'tree-attachment-audit.json'), cords=rows, body_visibility_hypotheses=tests, limitations=['Current physical cutouts are checked with their explicit image alpha and side rules.', 'A hidden cord is not proof of native attachment identity or physical feasibility.', 'This checks centerline visibility, not the full cord radius or all animated canopy phases.'])
        (base/'attachment-physical-occlusion-v2.json').write_text(json.dumps(result, indent=2)+'\n')
        print([(r['cord'], [(p['proposal']['camera_ray_shift'], p['samples'], p['exposed']) for p in r['hypotheses'][:3]]) for r in rows])
        print(tests)
    finally:
        release()


if __name__ == '__main__':
    main()
