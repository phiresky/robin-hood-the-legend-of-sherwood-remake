"""Prototype a continuous native-profile lower stem with a buried rear buttress."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY,replace_mesh
from refinement_workspace import _geometry
from join_tree07_stem import cut_upper,share_normals

def bvh(objects):
    vertices=[];faces=[]
    for obj in objects:
        start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        faces.extend(tuple(start+i for i in face.vertices) for face in obj.data.polygons)
    return BVHTree.FromPolygons(vertices,faces)

def main():
    old=tree_workspace(7);bank=scenery_workspace('croisement02-north-woodland-bank');out=OUT/'tree07-root-research/continuous-stem-v5';out.mkdir(parents=True,exist_ok=False)
    hashes={str(w):sha(w/'model.blend') for w in (old,bank)}
    bpy.ops.wm.open_mainfile(filepath=str(bank/'model.blend'));bpy.context.view_layer.update()
    support=bvh([o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==bank.name])
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];wood=next(o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==old.name and o.get('source_node')=='building-058' and o.get('projection_component')!='crown')
    upper=next(o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==old.name and o.get('source_node')=='building-062')
    preserved={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH' and o not in (wood,upper)}
    boundary,angles,junction=cut_upper(upper)
    row=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==7)
    canvas=Image.new('L',(1792,1152));canvas.paste(Image.open(OUT/'baseline/masks'/row['png']).convert('L'),tuple(row['box_top_left']));mask=np.asarray(canvas)>0
    ground=next(r['ground_y'] for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==7)
    vertices=[tuple(p) for p in boundary];profiles=[];n=len(angles);rows=[]
    for y in range(435,530):
        xs=np.where(mask[y])[0];xs=xs[(xs>=735)&(xs<=790)]
        if not len(xs):continue
        left,right=float(xs.min())-.45,float(xs.max())+.45;cx=(left+right)/2;radius=(right-left)/2
        rows.append((y,cx,radius))
    kernel=np.exp(-np.arange(-3,4,dtype=float)**2/(2*1.25**2));kernel/=kernel.sum()
    centers=np.convolve(np.pad([r[1] for r in rows],3,mode='edge'),kernel,mode='valid');radii=np.convolve(np.pad([r[2] for r in rows],3,mode='edge'),kernel,mode='valid')
    for (y,_,_),cx,radius in zip(rows,centers,radii):
        left,right=float(cx-radius),float(cx+radius)
        center=Vector((cx,-ground/SIN,(ground-y)/COS));depth=center.dot(RAY)
        fade=min(1.,max(0.,(y-505)/23));fade=fade*fade*(3-2*fade)
        for j in range(n):
            angle=angles[j];wave=math.sin(angle);x=cx+radius*math.cos(angle)
            source=Vector((x,-y*SIN,-y*COS));d=depth+radius*wave
            if wave>=0:
                origin=source+RAY*5000;hit=support.ray_cast(origin,-RAY)[0]
                if hit is not None and fade:d=max(d,hit.dot(RAY)+.3*fade)
            else:
                point=source+RAY*d;target=max(0.,(point.z-41.949)/RAY.z)
                d-=target*fade*(-wave)
            if y<445:d=min(d,(junction['cut']-.25-source.z)/RAY.z)
            vertices.append(tuple(source+RAY*d))
        profiles.append(dict(source_y=y,left=left,right=right,rear_support_fade=fade))
    faces=[tuple(reversed(range(n)))]
    for row in range(len(profiles)):
        for j in range(n):a=row*n+j;b=row*n+(j+1)%n;faces.append((a,b,b+n,a+n))
    faces.append(tuple(len(profiles)*n+j for j in range(n)))
    result=replace_mesh(wood,vertices,faces,materials=list(wood.data.materials))
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Continuous stem must be closed')
    for face in wood.data.polygons:face.use_smooth=True
    junction['shared_boundary_normals']=share_normals([wood,upper],junction['cut'])
    if preserved!={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH' and o not in (wood,upper)}:raise ValueError('Crown or other owner changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    for path,digest in hashes.items():
        if sha(Path(path)/'model.blend')!=digest:raise ValueError('Input changed')
    write_json(out/'evidence.json',dict(status='Private solid-only prototype; projection and local coverage not yet verified',model_sha256=sha(out/'model.blend'),inputs=hashes,geometry=result,source_profiles=profiles,preserved_appearance=preserved,changed_source_nodes=['building-058','building-062'],upper_junction=junction,limitations=['Main lower-stem tube fragments replaced by one continuous native-silhouette loft.','Rear root volume descends into bank0 at Z41.949, inferred from measured support.','Materials are provisional until reprojection; no texture-quality or geometry approval claimed.']))
    (out/'recipe.py').write_text(Path(__file__).read_text());(out/'join-recipe.py').write_text(Path(__file__).with_name('join_tree07_stem.py').read_text())
    import inspect_tree07_base
    previous_args=sys.argv;sys.argv=[sys.argv[0],'--','--model',str(out/'model.blend'),'--output-name',out.name+'-review','--solid-only']
    try:inspect_tree07_base.main()
    finally:sys.argv=previous_args
    print(out)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
