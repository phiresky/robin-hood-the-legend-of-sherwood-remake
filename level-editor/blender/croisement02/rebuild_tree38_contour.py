"""Private tree38 lower-contour continuation, excluding uncertain distal pixels."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
from rebuild_tree32_roots import loft,add_mesh,check
from tree_geometry import SIN,COS,RAY


def main():
    old=tree_workspace(38);out=OUT/'tree38-root-research/continuous-contour-v9';out.mkdir(parents=True,exist_ok=False);old_hash=sha(old/'model.blend');domain=OUT/'tree38-root-research/source-domain-v1'
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=next(o for o in objects if o.type=='MESH' and o.get('asset_group')==old.name and o.get('projection_component')!='crown')
    if wood.get('source_node')!='building-094':raise ValueError('Unexpected native wood owner')
    protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o!=wood};reference=wood.data.copy()
    for v in reference.vertices:v.co=wood.matrix_world@v.co
    mask=np.asarray(Image.open(domain/'wood38-plus-reviewed-contour.png').convert('L'))>0;ground=next(r['ground_y'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==38)
    profiles={};bm=bmesh.new()
    for role,start,end in [('stem',615,713)]:
        rows=[]
        for y in range(start,end):
            xs=np.where(mask[y])[0];xs=xs[(xs>=1550)&(xs<=1610)]
            if not len(xs):continue
            left,right=float(xs.min())-.35,float(xs.max())+1.35;rows.append((y+.5,(left+right)/2,(right-left)/2))
        if len(rows)<3:raise ValueError('Insufficient native root profile')
        profiles[role]=rows
        def adjust(y,cx,radius,default_depth):
            center=Vector((cx,-ground/SIN,(ground-y)/COS))
            # Keep the complete observed front half above ground; bury only the inferred rear.
            required=.25
            return center.dot(RAY)+(max(0.,(required-center.z)/RAY.z) if y>=687 else 0.)
        add_mesh(bm,loft(rows,ground,115,ground_start=687,depth_adjust=adjust))
    retained=bmesh.new();retained.from_mesh(reference)
    bmesh.ops.bisect_plane(retained,geom=list(retained.verts)+list(retained.edges)+list(retained.faces),dist=.0001,plane_co=(0,0,80),plane_no=(0,0,1),clear_inner=True)
    bmesh.ops.holes_fill(retained,edges=[e for e in retained.edges if e.is_boundary],sides=0);mesh=bpy.data.meshes.new('Retained upper38');retained.to_mesh(mesh);retained.free();bm.from_mesh(mesh);bpy.data.meshes.remove(mesh)
    mesh=bpy.data.meshes.new('Continuous tree38 lower contour');bm.to_mesh(mesh);bm.free();temporary=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(temporary)
    bpy.ops.object.select_all(action='DESELECT');temporary.select_set(True);bpy.context.view_layer.objects.active=temporary
    mod=temporary.modifiers.new('Continuous wood union','REMESH');mod.mode='VOXEL';mod.voxel_size=.6;mod.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=mod.name)
    group=temporary.vertex_groups.new(name='Lower joins')
    for v in temporary.data.vertices:group.add([v.index],max(0.,min(1.,(130-v.co.z)/30)),'REPLACE')
    mod=temporary.modifiers.new('Rounded joins','SMOOTH');mod.vertex_group=group.name;mod.factor=.3;mod.iterations=3;bpy.ops.object.modifier_apply(modifier=mod.name)
    mesh=temporary.data.copy();bpy.data.objects.remove(temporary,do_unlink=True);report=check(mesh);wood.data=mesh;wood.parent=None;wood.matrix_world=Matrix.Identity(4)
    neutral=bpy.data.materials.new('Unprojected private wood38');neutral.diffuse_color=(.4,.4,.4,1);mesh.materials.append(neutral)
    for f in mesh.polygons:f.use_smooth=True
    refobj=bpy.data.objects.new('Original38 exterior diagnostic',reference);bpy.context.scene.collection.objects.link(refobj);bpy.ops.object.select_all(action='DESELECT');refobj.select_set(True);bpy.context.view_layer.objects.active=refobj
    mod=refobj.modifiers.new('Original exterior union','REMESH');mod.mode='VOXEL';mod.voxel_size=.6;bpy.ops.object.modifier_apply(modifier=mod.name)
    target=BVHTree.FromPolygons([v.co for v in mesh.vertices],[list(f.vertices) for f in mesh.polygons]);source=BVHTree.FromPolygons([v.co for v in refobj.data.vertices],[list(f.vertices) for f in refobj.data.polygons]);distances=[target.find_nearest(v.co)[3] for v in refobj.data.vertices if v.co.z>140]+[source.find_nearest(v.co)[3] for v in mesh.vertices if v.co.z>140];bpy.data.objects.remove(refobj,do_unlink=True)
    if max(distances)>1.5:raise ValueError('Upper exterior drift: '+str(max(distances)))
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o!=wood}:raise ValueError('Crown or other asset changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    if sha(old/'model.blend')!=old_hash:raise ValueError('Approved input changed')
    write_json(out/'evidence.json',dict(model_sha256=sha(out/'model.blend'),previous_worker=str(old),previous_model_sha256=old_hash,source_review_sha256=sha(domain/'source-review.json'),geometry=report,profiles=profiles,upper_exterior_distance=dict(maximum=max(distances),p95=float(np.quantile(distances,.95))),preserved_appearance=protected,status='Private unprojected geometry; source and solid audits pending',limitations=['Only344 stronger contour pixels incorporated;65 distal-root-vs-ground pixels remain unresolved.','Native38 observed RGB not semantically reclassified as foliage.','No original worker or canonical selection changed.']))
    (out/'recipe.py').write_text(Path(__file__).read_text())
    import inspect_tree07_base
    previous=sys.argv;sys.argv=[sys.argv[0],'--','--mask','38','--model',str(out/'model.blend'),'--output-name','continuous-contour-v9-review','--solid-only']
    try:inspect_tree07_base.main()
    finally:sys.argv=previous
    import audit_tree38_contour
    sys.argv=[sys.argv[0],'--','--worker',str(out),'--preservation-base',str(old),'--output',str(out/'inspection')]
    try:audit_tree38_contour.main(release_slot=False)
    finally:sys.argv=previous

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
