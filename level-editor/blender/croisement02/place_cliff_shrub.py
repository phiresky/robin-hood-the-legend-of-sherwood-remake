"""Place hanging forest foliage at an explicitly inferred bank-crest attachment."""
import json,sys,numpy as np
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release
from opacity_bounds import measure
from tree_geometry import RAY,SIN,COS
from foliage_bank_support import load_support,lower_anchor,visible_points
from mathutils import Vector
from PIL import Image,ImageDraw

def main():
    index=63
    bank,bank_hash,support=load_support()
    asset=f'croisement02-shrub-{index}';previous=OUT/'understory-round-8/assets'/asset;worker=OUT/'understory-round-10/assets'/asset;folder=OUT/'understory-candidates/forest-cliff-support-v1'
    if worker.exists() or folder.exists():raise FileExistsError(worker)
    folder.mkdir(parents=True);oldhash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=1:raise ValueError('Expected one western foliage source')
    obj=objects[0];bounds=measure(obj);anchor,_=lower_anchor(obj);delta=RAY*57.
    for v in obj.data.vertices:v.co+=delta
    obj.data.update();after=measure(obj)
    if after['bounds_min'][2]<0 or abs(-delta.y*SIN-delta.z*COS)>1e-5:raise ValueError('Visible support or projection drift')
    root=Vector((float(anchor[0]),float(anchor[1]+delta.y+5.),0.));hits=[]
    for name,bvh in support:
        p,n,f,d=bvh.ray_cast(Vector((root.x,root.y,1000)),Vector((0,0,-1)))
        if p is not None:hits.append((p.z,name))
    if not hits:raise ValueError('Inferred attachment is outside bank')
    root.z,root_part=max(hits)
    # Prefer an actual dense-body leaf sample near the proposed attachment.
    # The attachment remains inferred; a leaf sample is not woody-root evidence.
    points=visible_points(obj);local=points[np.abs(points[:,2]-root.z)<4.]
    distances=np.linalg.norm(local[:,:2]-np.asarray(root)[:2],axis=1)
    selected=None
    for k in np.argsort(distances)[:1000]:
        if distances[k]>30:break
        p=local[k];local_hits=[]
        for name,bvh in support:
            hit,n,f,d=bvh.ray_cast(Vector((float(p[0]),float(p[1]),1000)),Vector((0,0,-1)))
            if hit is not None:local_hits.append((hit.z,name))
        if local_hits and abs(max(local_hits)[0]-root.z)<.05:
            root=Vector((float(p[0]),float(p[1]),max(local_hits)[0]));root_part=max(local_hits)[1];selected=p.tolist();break
    if selected is None:raise ValueError('No dense foliage adjacent to inferred plateau attachment')
    if not all(after['bounds_min'][i]<=root[i]<=after['bounds_max'][i] for i in range(3)):raise ValueError('Attachment is outside foliage volume bounds')
    native_root=[float(root.x),float(-root.y*SIN-root.z*COS)]
    catalog=json.loads((previous/'reference/grouping.json').read_text());group=next(g for g in catalog['groups'] if g['id']==asset);nodes={p['node'] for p in group['parts']}
    scoped=dict(catalog,groups=[group],canonical_owners={n:asset for n in nodes});write_json(folder/'catalog.json',scoped)
    manifest=json.loads((previous/'source-masks.json').read_text())
    for projection in manifest['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group')==asset];projection['occluder_constraints']=[]
    write_json(folder/'source-masks.json',manifest)
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old!=obj:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Exact registered-proposal source node and authored leaf domain, isolated for placement review. Foreign objects remain untouched in prior worker.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=json.loads((previous/'inspection/refinement.json').read_text());report.update(model_sha256=sha(worker/'model.blend'),status='Inferred bank-crest attachment with hanging lower foliage; actual joint review pending');report['crown']['opacity_bounds']=after;report['crown']['minimum_z']=min(v.co.z for v in obj.data.vertices)
    write_json(inspection/'refinement.json',report);write_json(inspection/'support-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),world_delta=list(delta),previous_opacity_bounds=bounds,current_opacity_bounds=after,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)],bank_worker=str(bank),bank_model_sha256=bank_hash,inferred_attachment_world=list(root),inferred_attachment_source=native_root,support_part=root_part,inferred_not_observed_wood=True,adjacent_visible_leaf_sample=selected,reason='Lowest source leaves project below the bank crest. Ray shift57 places hanging foliage in front of cliff; an inferred attachment5 units behind its lower fringe lies on the actual plateau. No observed woody stem claimed.'))
    crop=(404,383,589,564);image=Image.open(previous/'reference/source.png').convert('RGB').crop(crop).resize((740,724),Image.Resampling.NEAREST);draw=ImageDraw.Draw(image);x=(native_root[0]-crop[0])*4;y=(native_root[1]-crop[1])*4
    draw.ellipse((x-7,y-7,x+7,y+7),outline='#ff00ff',width=2);draw.text((8,8),'Inferred attachment; not observed stem evidence',fill='white');image.save(inspection/'inferred-attachment.png')
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(bank/'model.blend')!=bank_hash:raise ValueError('Bank changed during attachment review')
    if sha(previous/'model.blend')!=oldhash:raise ValueError('Previous candidate changed')
    print('SUPPORTED',index,sha(worker/'model.blend'),flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
