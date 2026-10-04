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
from tree_geometry import RAY
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
            proposal = cord['selected_for_review']
            start = Vector(cord['current_upper_world']) + RAY*proposal['camera_ray_shift']
            end = Vector(proposal['attachment_world'])
            samples = max(2, int((end-start).length*2))
            exposed = []
            for i in range(samples):
                point = start.lerp(end, i/(samples-1))
                hit = tree.ray_cast(point+RAY*3000, -RAY, 2999.99)
                if hit[0] is None:
                    exposed.append(dict(sample=i, world=list(point)))
            rows.append(dict(cord=cord['cord'], proposal=proposal, samples=samples, exposed=len(exposed), exposed_points=exposed))
        result = dict(status='Private attachment screening; no tree or net model changed', tree_bindings=bindings, attachment_audit_sha256=sha(base/'tree-attachment-audit.json'), cords=rows, limitations=['Current physical cutouts are checked with their explicit image alpha and side rules.', 'A hidden cord is not proof of native attachment identity or physical feasibility.', 'This checks centerline visibility, not the full cord radius or all animated canopy phases.'])
        (base/'attachment-physical-occlusion.json').write_text(json.dumps(result, indent=2)+'\n')
        print([(r['cord'], r['samples'], r['exposed']) for r in rows])
    finally:
        release()


if __name__ == '__main__':
    main()
