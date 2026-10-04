"""Reconstruct the southwest firewood as closed logs using its native footprint."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
ASSET='croisement03-southwest-firewood-stack'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def main():
    root=OUT/'firewood-candidate-v1';root.mkdir(exist_ok=True)
    worker=root/'assets'/ASSET
    if worker.exists():raise FileExistsError(worker)
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']: row['png']=str(OUT/'baseline/masks'/row['png'])
    write_json(root/'mask-inventory.json',masks)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Initial native firewood; obscured timber remains unknown',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[114],exclude_mask_indices=[45,54],exclusions_reviewed=True,exclusion_reason='Native source close-up shows foreground leaf pixels over timber. Foliage45 covers487 firewood-mask pixels and foliage54 covers11; these remain foliage-owned, with neighboring plant geometry still required.')])}))
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=256,height=256,framing_padding=1.20,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    obj=next(o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('source_node')=='building-049')
    record=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())['sight_obstacles'][49]
    xy=np.asarray([(p['x'],-p['y']/SIN) for p in record['points']]);center=xy.mean(axis=0)
    values,axes=np.linalg.eigh(np.cov((xy-center).T));u=axes[:,np.argmax(values)];v=np.array([-u[1],u[0]])
    length=float(np.ptp((xy-center)@u));width=float(np.ptp((xy-center)@v));height=float(np.mean([p['z_top'] for p in record['points']])/COS)
    c=Vector((*center,0));u=Vector((*u,0));v=Vector((*v,0));up=Vector((0,0,1))
    radius=min(height/5.5,width/7.5);verts=[];faces=[]
    logs=[]
    for row,count in enumerate([4,3,2]):
        for col in range(count):
            middle=c+v*((col-(count-1)/2)*radius*1.95)+up*(radius+row*radius*1.7)
            # Stacked billets share a length; small endpoint variation is inferred.
            delta=(col%2-.5)*radius*.2
            a=middle-u*(length/2+delta);b=middle+u*(length/2-delta)
            start=len(verts);n=12
            for p in (a,b):
                verts.extend(tuple(p+radius*(v*math.cos(j*math.tau/n)+up*math.sin(j*math.tau/n))) for j in range(n))
            faces.append(tuple(start+j for j in reversed(range(n))))
            faces.extend((start+j,start+(j+1)%n,start+n+(j+1)%n,start+n+j) for j in range(n))
            faces.append(tuple(start+n+j for j in range(n)))
            logs.append(dict(a=list(a),b=list(b),radius=radius))
    old=obj.data;mesh=bpy.data.meshes.new('Southwest firewood closed billets');mesh.from_pydata(verts,[],faces);mesh.update()
    for material in old.materials:mesh.materials.append(material)
    obj.data=mesh;obj.matrix_world.identity()
    import bmesh
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh)
    topology=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    assert topology['nonmanifold_edges']==0 and topology['degenerate_faces']==0
    modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    write_json(inspection/'construction.json',dict(status='private candidate; visual and source coverage review pending',model_sha256=sha(worker/'model.blend'),native_mask=114,native_obstacle=49,logs=logs,topology=topology,limitations=['Nine billets and hidden end profiles are inferred; compare visible log count against native artwork.','Native footprint includes grass around the stack; source mask114 alone receives observed texture.','Lighting is explicit provisional inspection lighting; map shadow calibration remains pending.']))
    release()
    print(worker)
if __name__=='__main__': main()
