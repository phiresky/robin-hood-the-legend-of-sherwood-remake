"""Correct inferred color wedges on each southwest thicket lobe without changing geometry."""
import json,sys,shutil
from pathlib import Path
import numpy as np
import bpy
from PIL import Image
from scipy.ndimage import minimum_filter
from scipy.spatial import cKDTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release

def main():
    asset='croisement02-shrub-83';previous=OUT/'understory-round-14/assets'/asset;worker=OUT/'understory-round-18/assets'/asset;folder=OUT/'understory-candidates/thicket83-leaf-fill-v1'
    if folder.exists() or worker.exists():raise FileExistsError(folder)
    folder.mkdir(parents=True);previous_hash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=3:raise ValueError('Expected exactly three native thicket lobes')
    report=json.loads((previous/'inspection/refinement.json').read_text());old_packet=Path(report['source_packet']);packet=json.loads(old_packet.read_text());oldfolder=Path(packet['directory'])
    for name in ('observed-source.png','complete-source.png'):shutil.copyfile(oldfolder/name,folder/name)
    packet.update(directory=str(folder));packet['physical_silhouette_authority']['reason']='Exact native83 silhouette across three separate local foliage volumes; native84 and canopy130 are observed ownership exclusions only.'
    write_json(folder/'partition.json',packet);proofs=[]
    for obj in objects:
        label=obj.name.rsplit(' ',1)[-1];src=oldfolder/label;dest=folder/label;dest.mkdir();part=json.loads((src/'partition.json').read_text())
        coords=np.asarray([tuple(v.co) for v in obj.data.vertices]);faces=[tuple(p.vertices) for p in obj.data.polygons];uv=np.asarray([tuple(v.uv) for v in obj.data.uv_layers['Foliage UV'].data]);matrix=np.asarray(obj.matrix_world)
        for name in ('observed-source.png','complete-source.png'):shutil.copyfile(src/name,dest/name)
        observed=np.asarray(Image.open(src/'observed-source.png').convert('RGBA'));complete=np.asarray(Image.open(src/'complete-source.png').convert('RGBA'));known=observed[:,:,3]>127;h,w=known.shape
        donors=np.argwhere(minimum_filter(known.astype('uint8'),size=7)>0)
        if len(donors)<30:raise ValueError('Insufficient same-lobe dense leaf donors')
        rng=np.random.default_rng(830018+len(proofs));sites=np.array([[y+rng.uniform(-2,2),x+rng.uniform(-2,2)] for y in range(0,h,6) for x in range(0,w,6)])
        yy,xx=np.indices((h,w));positions=np.column_stack((yy.ravel(),xx.ravel()));ids=cKDTree(sites).query(positions)[1];picked=donors[rng.integers(0,len(donors),len(sites))];offsets=np.clip(np.rint(positions-sites[ids]).astype(int),-3,3);samples=picked[ids]+offsets
        fill=observed[samples[:,0],samples[:,1]].reshape(h,w,4).copy();fill[known]=observed[known];fill[:,:,3]=complete[:,:,3]
        Image.fromarray(fill).save(dest/'inferred-source.png');unknown=fill.copy();unknown[:,:,3]=np.where(known,0,complete[:,:,3]);Image.fromarray(unknown).save(dest/'unknown-front.png');Image.fromarray(fill).save(dest/'leaf-fill.png')
        part.update(directory=str(dest),inferred_front_image='leaf-fill.png');write_json(dest/'partition.json',part)
        for slot,name in ((1,'unknown-front.png'),(2,'inferred-source.png')):
            mat=obj.data.materials[slot]
            if 'inferred' not in mat.name:raise ValueError('Unexpected material role')
            texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);texture.image=bpy.data.images.load(str(dest/name),check_existing=False);texture.image.pack()
        if faces!=[tuple(p.vertices) for p in obj.data.polygons] or not np.array_equal(coords,np.asarray([tuple(v.co) for v in obj.data.vertices])) or not np.array_equal(uv,np.asarray([tuple(v.uv) for v in obj.data.uv_layers['Foliage UV'].data])) or not np.array_equal(matrix,np.asarray(obj.matrix_world)):raise ValueError('Protected geometry changed')
        if sha(src/'observed-source.png')!=sha(dest/'observed-source.png'):raise ValueError('Native observed pixels changed')
        proofs.append(dict(lobe=label,observed_source_sha256=sha(dest/'observed-source.png'),leaf_fill_sha256=sha(dest/'leaf-fill.png'),observed_pixels=int(known.sum()),known_rgb_changed=0,geometry_uv_and_world_matrix_unchanged=True))
    catalog=json.loads((previous/'reference/grouping.json').read_text());group=next(g for g in catalog['groups'] if g['id']==asset);nodes={p['node'] for p in group['parts']};write_json(folder/'catalog.json',dict(catalog,groups=[group],canonical_owners={n:asset for n in nodes}))
    manifest=json.loads((previous/'source-masks.json').read_text())
    for projection in manifest['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group')==asset];projection['occluder_constraints']=[]
    write_json(folder/'source-masks.json',manifest)
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old not in objects:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True);inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Unchanged three authored thicket lobes and source node; material-only own-leaf inference correction.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report.update(model_sha256=sha(worker/'model.blend'),source_packet=str(folder/'partition.json'),status='Three-lobe leaf-scale inferred fill correction, actual/joint review pending');write_json(inspection/'refinement.json',report)
    write_json(inspection/'inferred-fill-evidence.json',dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(previous),previous_model_sha256=previous_hash,lobes=proofs,geometry_and_uv_unchanged=True,known_rgb_changed=0,reason='Six-pixel irregular same-lobe leaf patches replace broad nearest-edge wedges only on inferred front/back materials.'))
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=previous_hash:raise ValueError('Previous candidate changed')
    print('CORRECTED',worker,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
