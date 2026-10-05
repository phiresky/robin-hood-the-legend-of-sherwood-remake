"""Private bounded wood union to inspect segmented trunk and lateral root joins."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Matrix
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from rebuild_tree32_roots import check


def main():
    worker=tree_workspace(31);digest=sha(worker/'model.blend');out=OUT/'restart2-wood/tree31-union-v1';out.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects)
    wood={int(o['source_node'].split('-')[-1]):o for o in objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'}
    if set(wood)!={77,78,79}:raise ValueError('Unexpected native wood owners')
    protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()}
    merged=bmesh.new();points=[]
    for obj in wood.values():
        mesh=obj.data.copy();mesh.transform(obj.matrix_world);points.extend(v.co.copy() for v in mesh.vertices);merged.from_mesh(mesh);bpy.data.meshes.remove(mesh)
    mesh=bpy.data.meshes.new('Private tree31 continuous wood union');merged.to_mesh(mesh);merged.free();obj=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(obj)
    bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
    modifier=obj.modifiers.new('Resolve intersecting native tubes','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.45;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
    modifier=obj.modifiers.new('Relax intersection ridges','SMOOTH');modifier.factor=.2;modifier.iterations=2;bpy.ops.object.modifier_apply(modifier=modifier.name)
    combined=obj.data.copy();bpy.data.objects.remove(obj,do_unlink=True);full=check(combined)
    surface=BVHTree.FromPolygons([v.co for v in combined.vertices],[list(p.vertices) for p in combined.polygons]);distances=[surface.find_nearest(p)[3] for p in points]
    # Interior caps are not exterior-displacement evidence; retain the raw diagnostic without using it as a geometry guard.
    exterior_drift_note='Original overlapping tube caps can be interior to the union; raw nearest distances are diagnostic only. Exact outside geometry/material preservation remains required.'

    bm=bmesh.new();bm.from_mesh(combined);pending=set(bm.verts);components=[]
    while pending:
        seed=pending.pop();seen={seed};stack=[seed]
        while stack:
            v=stack.pop()
            for e in v.link_edges:
                other=e.other_vert(v)
                if other in pending:pending.remove(other);seen.add(other);stack.append(other)
        components.append(len(seen))
    bm.free()
    normals=KDTree(len(combined.vertices));normal_values=[]
    for vertex in combined.vertices:normals.insert(vertex.co,vertex.index);normal_values.append(tuple(vertex.normal))
    normals.balance()
    neutral=bpy.data.materials.new('Private unprojected wood31');neutral.diffuse_color=(.4,.4,.4,1);reports={}
    for part,obj in wood.items():
        bm=bmesh.new();bm.from_mesh(combined)
        bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,100),plane_no=(0,0,1),clear_inner=part!=77,clear_outer=part==77)
        bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        if part!=77:
            bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(960,0,0),plane_no=(1,0,0),clear_inner=part==78,clear_outer=part==79)
            bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new(f'Tree31 wood owner {part}');bm.to_mesh(mesh);bm.free();obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4);mesh.materials.append(neutral)
        for polygon in mesh.polygons:polygon.use_smooth=True
        mesh.normals_split_custom_set_from_vertices([normal_values[normals.find(v.co)[1]] for v in mesh.vertices]);reports[part]=check(mesh)
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()}:raise ValueError('Protected appearance changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    write_json(out/'evidence.json',dict(status='Private shape prototype; no source bake or approval',model_sha256=sha(out/'model.blend'),previous_worker=str(worker),previous_model_sha256=digest,full_geometry=full,parts=reports,connected_components=sorted(components,reverse=True),surface_deviation_note=exterior_drift_note,surface_deviation_max=max(distances),surface_deviation_p95=float(np.quantile(distances,.95)),protected_appearance=protected,limitations=['Part partition is internal ownership inference, not visible seams.','Disconnected remnants must be visually evaluated; union alone is not proof of continuity.','Native green/shadowed wood color must be retained at projection.']))
    (out/'recipe.py').write_text(Path(__file__).read_text())
    if sha(worker/'model.blend')!=digest:raise ValueError('Selected model changed')
    import restart2_inspect_wood
    previous=sys.argv;sys.argv=[sys.argv[0],'--','--mask','31','--model',str(out/'model.blend'),'--output',str(out/'ground-review'),'--solid-only']
    try:restart2_inspect_wood.main()
    finally:sys.argv=previous

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
