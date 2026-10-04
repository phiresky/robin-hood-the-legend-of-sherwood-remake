"""Supplement a tiny north fringe with adjacent original leaf donors while preserving observed pixels and geometry."""
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

def main(index=22):
    asset='croisement02-canopy-fringe-22';previous=OUT/'understory-round-21/assets'/asset;worker=OUT/'understory-round-23/assets'/asset;folder=OUT/'understory-candidates/north-fringe22-leaf-fill-v2'
    if worker.exists() or folder.exists():raise FileExistsError(worker)
    folder.mkdir(parents=True);oldhash=sha(previous/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset]
    if len(objects)!=1:raise ValueError('Expected one western foliage source')
    obj=objects[0];bounds=measure(obj)
    previous_report=json.loads((previous/'inspection/refinement.json').read_text());packet=json.loads(Path(previous_report['source_packet']).read_text());source_folder=Path(packet['directory'])
    old_coordinates=np.asarray([tuple(v.co) for v in obj.data.vertices]);old_faces=[tuple(p.vertices) for p in obj.data.polygons];old_uv=np.asarray([tuple(v.uv) for v in obj.data.uv_layers['Foliage UV'].data])
    for name in ('observed-source.png','complete-source.png'):
        shutil.copyfile(source_folder/name,folder/name)
    observed=np.asarray(Image.open(folder/'observed-source.png').convert('RGBA'));complete=np.asarray(Image.open(folder/'complete-source.png').convert('RGBA'));known=observed[:,:,3]>127
    rgb=complete[:,:,:3].astype(float);lum=rgb@np.array([.2126,.7152,.0722]);supplement=(complete[:,:,3]>127)&(lum>70)&(rgb[:,:,1]>.55*rgb[:,:,0]);supplement[:40]=False
    supplement_image=complete.copy();supplement_image[:,:,3]=supplement*255;Image.fromarray(supplement_image).save(folder/'supplementary-donors.png')
    dense=minimum_filter(supplement.astype('uint8'),size=3)>0
    donors=np.argwhere(dense);h,w=known.shape
    if len(donors)<30:raise ValueError('Insufficient dense native leaf donors')
    rng=np.random.default_rng(220023);sites=[]
    for y in range(0,h,3):
        for x in range(0,w,3):sites.append([y+rng.uniform(-1,1),x+rng.uniform(-1,1)])
    sites=np.asarray(sites);tree=cKDTree(sites);yy,xx=np.indices((h,w));positions=np.column_stack((yy.ravel(),xx.ravel()));ids=tree.query(positions)[1]
    picked=donors[rng.integers(0,len(donors),size=len(sites))]
    offsets=np.rint(positions-sites[ids]).astype(int);offsets=np.clip(offsets,-1,1);samples=picked[ids]+offsets
    samples[:,0]=np.clip(samples[:,0],0,h-1);samples[:,1]=np.clip(samples[:,1],0,w-1)
    fill=complete[samples[:,0],samples[:,1]].reshape(h,w,4).copy();fill[known]=observed[known];fill[:,:,3]=complete[:,:,3]
    Image.fromarray(fill).save(folder/'leaf-fill.png')
    packet.update(directory=str(folder),inferred_front_image='leaf-fill.png');write_json(folder/'partition.json',packet)
    unknown=fill.copy();unknown[:,:,3]=np.where(known,0,complete[:,:,3]);Image.fromarray(unknown).save(folder/'unknown-front.png');Image.fromarray(fill).save(folder/'inferred-source.png')
    for slot,name in ((1,'unknown-front.png'),(2,'inferred-source.png')):
        mat=obj.data.materials[slot]
        if 'inferred' not in mat.name:raise ValueError('Unexpected inferred material slot')
        texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        texture.image=bpy.data.images.load(str(folder/name),check_existing=False);texture.image.pack()
    # The old interior atlas has the same dark-donor bias. Change RGB only;
    # every alpha texel and every face/UV remains exactly as authored.
    atlas=np.asarray(Image.open(source_folder/'inferred-volume.png').convert('RGBA')).copy();ah,aw=atlas.shape[:2];gy,gx=np.indices((ah,aw));tile_y=gy//6;tile_x=gx//6
    choices=rng.integers(0,len(donors),size=((ah+5)//6,(aw+5)//6));centers=donors[choices[tile_y,tile_x]];sample_y=np.clip(centers[:,:,0]+(gy%6)//2-1,0,h-1);sample_x=np.clip(centers[:,:,1]+(gx%6)//2-1,0,w-1)
    old_alpha=atlas[:,:,3].copy();atlas[:,:,:3]=complete[sample_y,sample_x,:3];Image.fromarray(atlas).save(folder/'inferred-volume.png')
    if not np.array_equal(old_alpha,atlas[:,:,3]):raise ValueError('Inferred physical alpha changed')
    atlas_image=bpy.data.images.load(str(folder/'inferred-volume.png'),check_existing=False);atlas_image.pack()
    for slot in (3,4):
        mat=obj.data.materials[slot];texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);texture.image=atlas_image
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
        if old.type=='MESH' and old!=obj:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    inventory(folder/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=previous/'reference/source.png');validate_catalog(folder/'inventory/inventory.json',folder/'catalog.json')
    write_json(folder/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(folder/'catalog.json'),inventory_sha256=sha(folder/'inventory/inventory.json'),evidence='Exact registered-proposal source node and authored leaf domain, isolated for placement review. Foreign objects remain untouched in prior worker.'))
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',grouping_manifest=folder/'catalog.json',inventory_path=folder/'inventory/inventory.json',review_path=folder/'grouping-review.json',source_mask_manifest=folder/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    report=previous_report;report.update(model_sha256=sha(worker/'model.blend'),source_packet=str(folder/'partition.json'),status='Leaf-scale inferred texture correction; actual and bank joint review pending');report['crown'].update(opacity_bounds=after)
    write_json(inspection/'refinement.json',report);write_json(inspection/'inferred-fill-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=oldhash,model_sha256=sha(worker/'model.blend'),observed_source_sha256=sha(folder/'observed-source.png'),known_rgb_changed=0,geometry_and_uv_unchanged=True,previous_opacity_bounds=bounds,current_opacity_bounds=after,leaf_fill_sha256=sha(folder/'leaf-fill.png'),reason='Unique387 observed pixels are predominantly dark shadow and cannot alone define unseen crown foliage. Supplementary three-pixel golden/green donor patches come from the original native22 support already covered by existing134/135, with no new source ownership. Only inferred front/back/interior RGB changes; all observed source, alpha, geometry, topology and UVs are preserved.',supplementary_donors=str(folder/'supplementary-donors.png'),supplementary_donors_sha256=sha(folder/'supplementary-donors.png'),supplementary_native_pixel_count=int(supplement.sum()),inferred_physical_alpha_unchanged=True))
    if (previous/'inspection/support-evidence.json').exists():
        support=json.loads((previous/'inspection/support-evidence.json').read_text());support.update(model_sha256=sha(worker/'model.blend'),material_revision_previous_worker=str(previous),material_revision_previous_model_sha256=oldhash,material_revision_geometry_unchanged=True);write_json(inspection/'support-evidence.json',support)
        if (previous/'inspection/inferred-attachment.png').exists():shutil.copyfile(previous/'inspection/inferred-attachment.png',inspection/'inferred-attachment.png')
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=oldhash:raise ValueError('Previous candidate changed')
    print('SUPPORTED',index,sha(worker/'model.blend'),flush=True)
if __name__=='__main__':
    acquire()
    try:main(int(sys.argv[sys.argv.index('--')+1]) if '--' in sys.argv else 22)
    finally:release()
