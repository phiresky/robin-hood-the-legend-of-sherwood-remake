"""Repair curtain construction seams without changing its authored silhouette."""
import bpy
import bmesh

ASSET = 'derby-upper-west-curtain'
TAG = 'upper-west-curtain-round2-seams-v1'


def _stats(mesh):
    bm = bmesh.new()
    bm.from_mesh(mesh)
    result = dict(vertices=len(bm.verts), faces=len(bm.faces),
                  boundary_edges=sum(e.is_boundary for e in bm.edges),
                  nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),
                  degenerate_faces=sum(f.calc_area() < 1e-6 for f in bm.faces))
    bm.free()
    return result


def refine():
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and o.get('asset_group') == ASSET and not o.hide_render]
    if all(o.get('round2_upper_west') == TAG for o in objects):
        return {'reused': True}
    report = {}
    stair = next(o for o in objects if o.get('source_node') == 'building-114' and not o.get('step_count'))
    treads = next(o for o in objects if o.get('source_node') == 'building-114' and o.get('step_count'))
    report['stair'] = {'before_support': _stats(stair.data), 'before_treads': _stats(treads.data)}
    # The closed tread solid already extends to the ground and includes both cheeks.
    # Keep the canonical part identity and discard the overlapping ramp skin.
    stair.data = treads.data.copy()
    stair.data.transform(stair.matrix_world.inverted() @ treads.matrix_world)
    stair['step_count'] = treads['step_count']
    objects.remove(treads)
    bpy.data.objects.remove(treads, do_unlink=True)
    report['stair']['after'] = _stats(stair.data)
    for obj in objects:
        node = obj.get('source_node')
        if node in {'building-074', 'building-116', 'building-127', 'building-128'}:
            before = _stats(obj.data)
            obj.data = obj.data.copy()
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            # Separately authored face boundaries differ by fractions of one map pixel.
            # Weld only within this subpixel tolerance; architectural corners stay fixed.
            bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=0.65)
            bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=0.0001)
            boundary = [e for e in bm.edges if e.is_boundary]
            # Ground caps are unseen construction closure, never inferred roof geometry.
            ground = [e for e in boundary if all(abs((obj.matrix_world @ v.co).z) < 0.01 for v in e.verts)]
            if ground:
                bmesh.ops.holes_fill(bm, edges=ground, sides=0)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bm.to_mesh(obj.data)
            bm.free()
            obj.data.update()
            report[node] = {'before': before, 'after': _stats(obj.data)}
        obj['round2_upper_west'] = TAG
    bpy.context.view_layer.update()
    return report
