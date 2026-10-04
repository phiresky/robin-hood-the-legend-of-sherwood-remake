"""Complete a bounded native trunk contour and separately inferred distal root."""
import json,math,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from rebuild_tree32_roots import add_mesh,check
from tree_geometry import SIN,COS,RAY


def loft(rows,ground,cut,n=48,ground_start=715):
    vertices=[];faces=[]
    kernel=np.array([1,2,3,2,1],float);kernel/=kernel.sum()
    centers=np.convolve(np.pad([r[1] for r in rows],2,mode='edge'),kernel,mode='valid')
    radii=np.convolve(np.pad([r[2] for r in rows],2,mode='edge'),kernel,mode='valid')
    for (y,_,_),cx,radius in zip(rows,centers,radii):
        center=Vector((cx,-ground/SIN,(ground-y)/COS));depth=center.dot(RAY)
        if y>=ground_start:depth+=max(0.,(.25+radius*RAY.z-center.z)/RAY.z)
        for j in range(n):
            angle=math.tau*j/n;source=Vector((cx+radius*math.cos(angle),-y*SIN,-y*COS));d=depth+radius*math.sin(angle)
            d=min(d,(cut-.25-source.z)/RAY.z)
            vertices.append(tuple(source+RAY*d))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(rows)-1):
        for j in range(n):a=i*n+j;b=i*n+(j+1)%n;faces.append((a,b,b+n,a+n))
    faces.append(tuple((len(rows)-1)*n+j for j in range(n)))
    return vertices,faces


def main():
    worker=tree_workspace(38);digest=sha(worker/'model.blend');output=OUT/'tree38-root-research/contour-prototype-v1';output.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
    if len(wood)!=1 or wood[0].get('source_node')!='building-094':raise ValueError('Unexpected tree38 wood')
    obj=wood[0];protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o!=obj}
    native=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==38)
    image=Image.new('L',(1792,1152));image.paste(Image.open(OUT/'baseline/masks'/native['png']),tuple(native['box_top_left']));domain=np.asarray(image)>0
    review=json.loads((OUT/'tree38-root-research/rgb-contour-review.json').read_text())
    trunkmask=np.asarray(Image.open(OUT/'tree38-root-research/trunk-basal-contour.png').convert('L'))>0
    rootmask=np.asarray(Image.open(OUT/'tree38-root-research/distal-root-inference.png').convert('L'))>0
    if int(trunkmask.sum())!=344 or int(rootmask.sum())!=65:raise ValueError('Inference classes changed')
    ground=next(r['ground_y'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==38)
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.transform(bm,matrix=obj.matrix_world,verts=list(bm.verts));bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,65),plane_no=(0,0,1),clear_inner=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0)
    profiles={}
    for role,mask,start in [('trunk-and-basal-contour',domain|trunkmask,635),('inferred-distal-root',rootmask,0)]:
        rows=[]
        for y in range(start,730):
            xs=np.where(mask[y])[0]
            if len(xs)<1:continue
            left,right=float(xs.min())-.65,float(xs.max())+.65;rows.append((y,(left+right)/2,(right-left)/2))
        if len(rows)<2:raise ValueError('Insufficient native profile')
        profiles[role]=rows;add_mesh(bm,loft(rows,ground,100,ground_start=697))
    mesh=bpy.data.meshes.new('Tree38 completed lower contour');bm.to_mesh(mesh);bm.free();temp=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(temp)
    bpy.ops.object.select_all(action='DESELECT');temp.select_set(True);bpy.context.view_layer.objects.active=temp
    modifier=temp.modifiers.new('Continuous native contour union','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.35;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
    combined=temp.data.copy();bpy.data.objects.remove(temp,do_unlink=True);obj.data=combined;obj.parent=None;obj.matrix_world=Matrix.Identity(4)
    neutral=bpy.data.materials.new('Private unprojected wood38');neutral.diffuse_color=(.4,.4,.4,1);combined.materials.append(neutral)
    for face in combined.polygons:face.use_smooth=True
    geometry=check(combined)
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o!=obj}:raise ValueError('Other appearance changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'model.blend'))
    write_json(output/'evidence.json',dict(status='Private unprojected shape hypothesis; source and oblique review required',model_sha256=sha(output/'model.blend'),previous_worker=str(worker),previous_model_sha256=digest,geometry=geometry,profiles=profiles,source_review=review,protected_appearance=protected,limitations=['344 native trunk/basal contour pixels and65 olive diagonal pixels have separate inference confidence.','65-pixel distal continuation could include root shadow or ground; source RGB must remain unchanged and no bark color may be fabricated over it.','No canonical selection or approval inherited.']))
    (output/'recipe.py').write_text(Path(__file__).read_text())
    if sha(worker/'model.blend')!=digest:raise ValueError('Selected model changed')

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
