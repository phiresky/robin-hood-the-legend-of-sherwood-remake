"""Remove recognizable mirrored branches from inferred off-map shrub texture."""
import json,sys, numpy as np, shutil
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
from shrub_geometry import build
from PIL import Image
from scipy.ndimage import minimum_filter
from scipy.spatial import cKDTree

def main(index=62):
    asset='croisement02-west-shrub-bank';previous=OUT/'understory-round-6/assets'/asset;worker=OUT/'understory-round-7/assets'/asset;folder=OUT/'understory-candidates/west-bank-v7'
    if worker.exists() or folder.exists():raise FileExistsError(worker)
    folder.mkdir(parents=True);oldhash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=2:raise ValueError('Expected two bank lobes')
    obj=min(objects,key=lambda o:min(v.co.x for v in o.data.vertices));bounds=measure(obj)
    previous_report=json.loads((previous/'inspection/refinement.json').read_text());packet=json.loads((OUT/'understory-candidates/west-bank-v5/west/partition.json').read_text());source_folder=Path(packet['directory'])
    old_coordinates=np.asarray([tuple(v.co) for v in obj.data.vertices]);old_faces=[tuple(p.vertices) for p in obj.data.polygons];old_uv=np.asarray([tuple(v.uv) for v in obj.data.uv_layers['Foliage UV'].data])
    for name in ('observed-source.png','complete-source.png'):
        shutil.copyfile(source_folder/name,folder/name)
    observed=np.asarray(Image.open(folder/'observed-source.png').convert('RGBA'));complete=np.asarray(Image.open(folder/'complete-source.png').convert('RGBA'));known=observed[:,:,3]>127
    leaf=known & (observed[:,:,1]>observed[:,:,0]*1.02) & (observed[:,:,1]>observed[:,:,2]*1.2)
    dense=(minimum_filter(known.astype('uint8'),size=7)>0)&leaf
    donors=np.argwhere(dense);h,w=known.shape
    if len(donors)<30:raise ValueError('Insufficient dense native leaf donors')
    rng=np.random.default_rng(620011);sites=[]
    for y in range(0,h,6):
        for x in range(0,w,6):sites.append([y+rng.uniform(-2,2),x+rng.uniform(-2,2)])
    sites=np.asarray(sites);tree=cKDTree(sites);yy,xx=np.indices((h,w));positions=np.column_stack((yy.ravel(),xx.ravel()));ids=tree.query(positions)[1]
    picked=donors[rng.integers(0,len(donors),size=len(sites))]
    offsets=np.rint(positions-sites[ids]).astype(int);offsets=np.clip(offsets,-3,3);samples=picked[ids]+offsets
    fill=observed[samples[:,0],samples[:,1]].reshape(h,w,4).copy();fill[known]=observed[known];fill[:,:,3]=complete[:,:,3]
    Image.fromarray(fill).save(folder/'leaf-fill.png')
    packet.update(directory=str(folder),inferred_front_image='leaf-fill.png');write_json(folder/'partition.json',packet)
    unknown=fill.copy();unknown[:,:,3]=np.where(known,0,complete[:,:,3]);Image.fromarray(unknown).save(folder/'unknown-front.png');Image.fromarray(fill).save(folder/'inferred-source.png')
    for slot,name in ((1,'unknown-front.png'),(2,'inferred-source.png')):
        mat=obj.data.materials[slot]
        if 'inferred' not in mat.name:raise ValueError('Unexpected inferred material slot')
        texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        texture.image=bpy.data.images.load(str(folder/name),check_existing=False);texture.image.pack()
    if old_faces!=[tuple(p.vertices) for p in obj.data.polygons] or not np.array_equal(old_uv,np.asarray([tuple(v.uv) for v in obj.data.uv_layers['Foliage UV'].data])) or not np.array_equal(old_coordinates,np.asarray([tuple(v.co) for v in obj.data.vertices])):raise ValueError('Material revision changed mesh topology/UV/vertices')
    obj.data.update();after=measure(obj)
    if not np.array_equal(observed[known],np.asarray(Image.open(folder/'observed-source.png'))[known]):raise ValueError('Observed source changed')
    catalog=json.loads((previous/'reference/grouping.json').read_text());group=next(g for g in catalog['groups'] if g['id']==asset);nodes={p['node'] for p in group['parts']}
    scoped=dict(catalog,groups=[group],canonical_owners={n:asset for n in nodes});write_json(folder/'catalog.json',scoped)
    manifest=json.loads((previous/'source-masks.json').read_text())
    for projection in manifest['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group')==asset];projection['occluder_constraints']=[]
    write_json(folder/'source-masks.json',manifest)
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old not in objects:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Exact registered-proposal source node and authored leaf domain, isolated for placement review. Foreign objects remain untouched in prior worker.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=previous_report;report.update(model_sha256=sha(worker/'model.blend'),status='Off-map mirrored branch replaced by inferred leaf patches; actual and renewed rock joint pending');report['crown']['lobes'][0]['opacity_bounds']=after
    write_json(inspection/'refinement.json',report);write_json(inspection/'inferred-fill-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),observed_source_sha256=sha(folder/'observed-source.png'),known_rgb_changed=0,geometry_and_uv_unchanged=True,previous_opacity_bounds=bounds,current_opacity_bounds=after,leaf_fill_sha256=sha(folder/'leaf-fill.png'),reason='Six-pixel irregular patches sampled from dense green native foliage replace mirrored recognizable pink branches in inferred off-map texture only. Observed source, mesh vertices, topology and UV coordinates preserved.'))
    support=json.loads((previous/'inspection/support-evidence.json').read_text());support.update(model_sha256=sha(worker/'model.blend'),material_revision_previous_worker=str(previous),material_revision_previous_model_sha256=oldhash,material_revision_geometry_unchanged=True);write_json(inspection/'support-evidence.json',support)
    write_json(folder/'revision.json',dict(previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),reason='Material-only off-map inferred branch correction; support and observed native art unchanged.'))
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=oldhash:raise ValueError('Previous candidate changed')
    print('MATERIAL REVISED',asset,sha(worker/'model.blend'),flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
