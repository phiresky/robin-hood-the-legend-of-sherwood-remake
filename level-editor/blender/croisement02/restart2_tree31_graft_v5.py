"""Join a continuous native-radius lower volume to the retained tree31 branches."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Matrix
from mathutils.kdtree import KDTree
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from rebuild_tree32_roots import check


def main():
    worker=tree_workspace(31);digest=sha(worker/'model.blend');out=OUT/'restart2-wood/tree31-sdf-v5'
    if (out/'model.blend').exists():raise FileExistsError(out/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood={int(o['source_node'].split('-')[-1]):o for o in objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'}
    if set(wood)!={77,78,79}:raise ValueError('Unexpected wood')
    protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()};reference=bmesh.new()
    for obj in wood.values():
        mesh=obj.data.copy();mesh.transform(obj.matrix_world);reference.from_mesh(mesh);bpy.data.meshes.remove(mesh)
    refmesh=bpy.data.meshes.new('Original31 reference');reference.to_mesh(refmesh);oldsurface=BVHTree.FromPolygons([v.co for v in refmesh.vertices],[list(p.vertices) for p in refmesh.polygons])
    bmesh.ops.bisect_plane(reference,geom=list(reference.verts)+list(reference.edges)+list(reference.faces),dist=.0001,plane_co=(0,0,80),plane_no=(0,0,1),clear_inner=True);bmesh.ops.holes_fill(reference,edges=[e for e in reference.edges if e.is_boundary],sides=0)
    uppermesh=bpy.data.meshes.new('Retained31 upper');reference.to_mesh(uppermesh);reference.free();upper=bpy.data.objects.new(uppermesh.name,uppermesh);bpy.context.scene.collection.objects.link(upper)
    data=np.load(out/'lower-volume.npz');mesh=bpy.data.meshes.new('Continuous31 lower');mesh.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());mesh.update();lower=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(lower)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=1e-5);bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=1e-5);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();check(mesh)
    bpy.ops.object.select_all(action='DESELECT');lower.select_set(True);bpy.context.view_layer.objects.active=lower
    modifier=lower.modifiers.new('Continuous lower and retained upper union','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.use_self=True;modifier.object=upper;bpy.ops.object.modifier_apply(modifier=modifier.name)
    combined=lower.data.copy();bpy.data.objects.remove(lower,do_unlink=True);bpy.data.objects.remove(upper,do_unlink=True)
    bm=bmesh.new();bm.from_mesh(combined);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(combined);bm.free();combined.update();full=check(combined)
    distances=[oldsurface.find_nearest(v.co)[3] for v in combined.vertices if v.co.z>140]
    if max(distances)>1.5:raise ValueError('Retained upper surface moved beyond bounded tolerance: '+str(max(distances)))
    normals=KDTree(len(combined.vertices));values=[]
    for v in combined.vertices:normals.insert(v.co,v.index);values.append(tuple(v.normal))
    normals.balance();neutral=bpy.data.materials.new('Private unprojected continuous31');neutral.diffuse_color=(.4,.4,.4,1);reports={}
    for part,obj in wood.items():
        bm=bmesh.new();bm.from_mesh(combined);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,100),plane_no=(0,0,1),clear_inner=part!=77,clear_outer=part==77);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        if part!=77:
            bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(960,0,0),plane_no=(1,0,0),clear_inner=part==78,clear_outer=part==79);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new(f'Continuous31 owner{part}');bm.to_mesh(mesh);bm.free();
        for attr in list(mesh.attributes):
            if attr.name.startswith('reprojection_'):mesh.attributes.remove(attr)
        obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4);mesh.materials.append(neutral)
        for f in mesh.polygons:f.use_smooth=True
        mesh.normals_split_custom_set_from_vertices([values[normals.find(v.co)[1]] for v in mesh.vertices]);reports[part]=check(mesh)
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood.values()}:raise ValueError('Protected outside changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    write_json(out/'evidence.json',dict(status='Private continuous lower volume; source and actual review pending',model_sha256=sha(out/'model.blend'),previous_worker=str(worker),previous_model_sha256=digest,full_geometry=full,parts=reports,upper_surface_to_old_distance_max=max(distances),protected_appearance=protected,volume_evidence_sha256=sha(out/'sdf-evidence.json'),limitations=['Native centreline radii constrain lower shape; transverse depth and blends inferred.','Upper branch surface remains within1.5 world units of original mesh; source bake not yet current.']))
    import restart2_inspect_wood
    prior=sys.argv;sys.argv=[sys.argv[0],'--','--mask','31','--model',str(out/'model.blend'),'--output',str(out/'ground-review'),'--solid-only']
    try:restart2_inspect_wood.main()
    finally:sys.argv=prior
    if sha(worker/'model.blend')!=digest:raise ValueError('Input changed')

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
