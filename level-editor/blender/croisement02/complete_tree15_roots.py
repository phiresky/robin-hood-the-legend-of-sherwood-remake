"""Create an isolated tree15 root continuation above its reviewed terrain bank."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified,validate
from tree_geometry import SIN,RAY,replace_mesh
from audit_candidates import audit
from render_tree import render_workspace


def bvh(objects):
    vertices=[];faces=[]
    for obj in objects:
        start=len(vertices);vertices.extend(obj.matrix_world @ v.co for v in obj.data.vertices)
        faces.extend(tuple(start+i for i in p.vertices) for p in obj.data.polygons)
    return BVHTree.FromPolygons(vertices,faces)


def root_geometry(wood,bank_bvh):
    wood_bvh=bvh([wood]);row=next(r for r in json.loads((OUT/'scenery-domains/inventory.json').read_text())['masks'] if r['index']==15)
    canvas=Image.new('L',(1792,1152));canvas.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']))
    left,top,right,bottom=1398,180,1439,210
    mask=np.asarray(canvas.crop((left,top,right,bottom)))>0
    # Resolve diagonal-only pixel contacts before constructing a watertight
    # root solid. These subpixel joins are inference, never source RGB evidence.
    for _ in range(3):
        additions=np.zeros_like(mask)
        for y in range(mask.shape[0]-1):
            for x in range(mask.shape[1]-1):
                tile=mask[y:y+2,x:x+2]
                if tile.sum()==2 and tile[0,0]==tile[1,1]:additions[y:y+2,x:x+2]=True
        mask|=additions
    connected=np.zeros_like(mask)
    pending=[(1415-left,190-top)]
    while pending:
        x,y=pending.pop()
        if not (0<=x<mask.shape[1] and 0<=y<mask.shape[0]) or connected[y,x] or not mask[y,x]:continue
        connected[y,x]=True
        pending.extend([(x+1,y),(x-1,y),(x,y+1),(x,y-1)])
    mask=connected
    coords=set()
    for y,x in zip(*np.where(mask)):
        coords.update([(int(x),int(y)),(int(x+1),int(y)),(int(x+1),int(y+1)),(int(x),int(y+1))])
    coords=sorted(coords);lookup={p:i for i,p in enumerate(coords)};depths={}
    for x,y in coords:
        origin=Vector((left+x,-(top+y)/SIN,0))+RAY*5000
        hits=[tree.ray_cast(origin,-RAY)[0] for tree in [wood_bvh,bank_bvh]]
        hits=[p for p in hits if p is not None]
        if hits:depths[x,y]=max(p.dot(RAY) for p in hits)+1.25
    for _ in range(60):
        pending={}
        for x,y in coords:
            if (x,y) in depths:continue
            near=[depths[x+dx,y+dy] for dx,dy in [(0,1),(0,-1),(1,0),(-1,0)] if (x+dx,y+dy) in depths]
            if near:pending[x,y]=float(np.mean(near))
        depths.update(pending)
    if len(depths)!=len(coords):raise ValueError('Root patch has unsupported disconnected source vertices')
    # A local upper envelope prevents the new bark from falling below the bank
    # between vertex samples, while retaining a restrained buttress thickness.
    smoothed={}
    for x,y in coords:
        near=[depths[x+dx,y+dy] for dx in range(-1,2) for dy in range(-1,2) if (x+dx,y+dy) in depths]
        smoothed[x,y]=max(depths[x,y],float(np.mean(near)))
    vertices=[]
    for rear in [False,True]:
        for x,y in coords:
            source=Vector((left+x,-(top+y)*SIN,-(top+y)*np.cos(np.radians(35))))
            vertices.append(tuple(source+RAY*(smoothed[x,y]-(8 if rear else 0))))
    n=len(coords);faces=[]
    for y,x in zip(*np.where(mask)):
        a,b,c,d=[lookup[p] for p in [(x,y),(x+1,y),(x+1,y+1),(x,y+1)]]
        faces.extend([(d,c,b,a),(a+n,b+n,c+n,d+n)])
        for dy,dx,p,q in [(0,-1,d,a),(-1,0,a,b),(0,1,b,c),(1,0,c,d)]:
            yy,xx=y+dy,x+dx
            if not (0<=yy<mask.shape[0] and 0<=xx<mask.shape[1] and mask[yy,xx]):faces.append((p,q,q+n,p+n))
    obj=wood.copy();obj.data=wood.data.copy();obj.name='Northeast Tree 15 / Observed basal-root continuation'
    bpy.data.collections['Croisement02 Working'].objects.link(obj)
    obj['part_name']='Observed basal-root continuation';obj['root_completion']=True
    result=replace_mesh(obj,vertices,faces,materials=list(wood.data.materials))
    for face in obj.data.polygons:face.use_smooth=True
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Root continuation must be closed and nondegenerate')
    result.update(source_crop=[left,top,right,bottom],source_node='building-033',native_mask=15,
        method='Closed root buttress following native bark silhouette above both existing wood and bank surfaces',
        inference='Eight-unit buried thickness and local depth envelope are inferred; observed source RGB remains authoritative')
    return obj,result


def root_projection_manifest(old, destination):
    """Retain visible native roots against overlapping coarse context proxies."""
    folder=destination/'root-source';folder.mkdir(parents=True,exist_ok=True)
    manifest=json.loads((old/'source-masks.json').read_text())
    inventory=json.loads(Path(manifest['mask_inventory']).read_text())
    native=next(row for row in inventory['masks'] if row['index']==15)
    image=Image.new('L',(1792,1152));image.paste(Image.open(native['png']).convert('L'),tuple(native['box_top_left']))
    local=np.zeros((1152,1792),dtype=bool);local[180:210,1398:1439]=np.asarray(image)[180:210,1398:1439]>0
    path=folder/'foreign-context-domain.png';Image.fromarray((~local).astype('uint8')*255).save(path)
    inventory['masks'].append(dict(index=372,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance='Complement of native15 basal-root artwork inside the reviewed continuation crop.'))
    write_json(folder/'inventory.json',inventory);manifest['mask_inventory']=str(folder/'inventory.json')
    manifest['projections']['exterior']['occluder_constraints']=[dict(reviewed=True,source_node=node,receiver_nodes=['building-033'],mask_indices=[372],reason='Native15 basal-root artwork is visibly owned by this tree; coarse neighbouring proxies must not reject its observed source texels.',review_evidence=str(OUT/'root-bank-source-revision/ownership-review.json')) for node in ['ground',*(f'building-{i:03}' for i in range(150))] if node!='building-033']
    write_json(folder/'assignments.json',manifest)
    return folder/'assignments.json'


def main(destination):
    old=tree_workspace(15);old_hash=sha(old/'model.blend');worker=destination/'assets'/old.name
    if worker.exists():raise ValueError('Use a fresh root-completion destination')
    bank=scenery_workspace('croisement02-northeast-oak-root-bank');bank_hash=sha(bank/'model.blend')
    cfg=json.loads((old/'workspace.json').read_text());source_manifest=root_projection_manifest(old,destination);acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank/'model.blend'))
        bank_tree=bvh([o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==bank.name])
        bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
        prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',
            grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',
            source_mask_manifest=source_manifest,width=384,height=384,framing_padding=1.7,lighting=cfg['lighting'])
        bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==old.name]
        wood=next(o for o in objects if o.get('projection_component')!='crown')
        saved={o.name:o.data.copy() for o in objects}
        for mesh in saved.values():
            for i,mat in enumerate(mesh.materials):mesh.materials[i]=mat.copy()
        obj,result=root_geometry(wood,bank_tree)
        bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
        for original in objects:original.data=saved[original.name]
        validate(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        (worker/'inspection').mkdir(exist_ok=True)
        report=json.loads((old/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),root_completion=result,status='New root-completion candidate; prior geometry approval does not apply')
        report['limitations'].append('Root continuation addresses local native15 coverage and bank contact; it requires its own geometry approval and later unseen texture completion.')
        write_json(worker/'inspection/refinement.json',report);audit(worker)
        new_name = next(o.name for o in bpy.data.collections['Croisement02 Working'].all_objects if o.get('root_completion'))
        preserved_names = [o.name for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==old.name and not o.get('root_completion')]
        render_workspace(worker,384,release_slot=False)
        write_json(worker/'inspection/root-completion.json',dict(previous_worker=str(old),previous_model_sha256=old_hash,model_sha256=sha(worker/'model.blend'),
            bank_worker=str(bank),bank_model_sha256=bank_hash,previous_meshes_preserved=list(saved),new_mesh=new_name,approval='pending'))
        if sha(old/'model.blend')!=old_hash or sha(bank/'model.blend')!=bank_hash:raise ValueError('Approved input or bank changed during candidate build')
    finally:release()


def resume(destination):
    old=tree_workspace(15);old_hash=sha(old/'model.blend');worker=destination/'assets'/old.name
    bank=scenery_workspace('croisement02-northeast-oak-root-bank');bank_hash=sha(bank/'model.blend')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==old.name]
        root=next(o for o in objects if o.get('root_completion'))
        validate(worker);(worker/'inspection').mkdir(exist_ok=True)
        result=dict(source_node='building-033',native_mask=15,source_crop=[1398,180,1439,210],vertices=len(root.data.vertices),faces=len(root.data.polygons),
                    method='Closed root buttress following native bark silhouette above existing wood and bank surfaces')
        report=json.loads((old/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),root_completion=result,status='New root-completion candidate; approval pending')
        write_json(worker/'inspection/refinement.json',report);audit(worker)
        new_name = next(o.name for o in bpy.data.collections['Croisement02 Working'].all_objects if o.get('root_completion'))
        preserved_names = [o.name for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==old.name and not o.get('root_completion')]
        render_workspace(worker,384,release_slot=False)
        write_json(worker/'inspection/root-completion.json',dict(previous_worker=str(old),previous_model_sha256=old_hash,model_sha256=sha(worker/'model.blend'),
            bank_worker=str(bank),bank_model_sha256=bank_hash,previous_meshes_preserved=preserved_names,new_mesh=new_name,approval='pending'))
        if sha(old/'model.blend')!=old_hash or sha(bank/'model.blend')!=bank_hash:raise ValueError('Input changed during root review')
    finally:release()


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('destination',type=Path);parser.add_argument('--resume-built',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    (resume if args.resume_built else main)(args.destination.resolve())
