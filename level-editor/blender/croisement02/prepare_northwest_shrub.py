"""Prepare native54 as a full-depth boundary shrub with inferred western completion."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from shrub_geometry import build
from audit_candidates import audit
from render_tree import render_workspace
DIRECTORY=OUT/'understory-candidates/northwest54-v1'
ASSET='croisement02-northwest-boundary-shrub-54'
NODE='foliage-shrub-054'


def main():
    DIRECTORY.mkdir(exist_ok=False)
    previous=OUT/'ground-plant-integration'
    catalog=json.loads(reviewed_catalog().read_text());write_json(DIRECTORY/'previous-catalog.json',catalog)
    if catalog!=json.loads((previous/'catalog.json').read_text()):raise ValueError('Reconcile current catalog with complete ground-plant source base')
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    manifest=json.loads((previous/'source-masks.json').read_text());inventory_path=Path(manifest['mask_inventory'])
    native=json.loads(inventory_path.read_text());rows={r['index']:r for r in native['masks']}
    def canvas(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];result=np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w]=np.asarray(Image.open(inventory_path.parent/row['png']).convert('L'))>0
        return result
    union=canvas(54)
    domain=DIRECTORY/'domain-417.png';Image.fromarray(union.astype('uint8')*255).save(domain)
    native['masks'].append(dict(index=417,layer=0,png=str(domain),box_top_left=[0,0],box_size=[1792,1152],provenance='Exact native54 observed pixels; off-map completion is inferred only.'))
    catalog['groups'].append(dict(id=ASSET,name='Northwest Boundary Shrub54',authored_scenery=True,native_foliage_mask=54,parts=[dict(node=NODE,name='Northwest foreground shrub lobes',foliage_domain_mask=417)]))
    catalog['canonical_owners'][NODE]=ASSET
    exterior=manifest['projections']['exterior'];exterior['assignments'].append(dict(source_node=NODE,mask_indices=[417],reviewed=True))
    receivers={'ground',*catalog['canonical_owners']}
    for rule in exterior.get('occluder_constraints',[]):
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    exterior.setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=NODE,receiver_nodes=sorted(receivers-{NODE}),mask_indices=[417],reason='Inferred shrub volume only occludes foreign source pixels inside the native54 domain.'))
    ground=next(r for r in exterior['assignments'] if r.get('source_node')=='ground');ground['exclude_mask_indices'].append(417)
    write_json(DIRECTORY/'catalog.json',catalog);write_json(DIRECTORY/'mask-inventory.json',native)
    manifest['mask_inventory']=str(DIRECTORY/'mask-inventory.json');write_json(DIRECTORY/'source-masks.json',manifest)
    # One complete observed union is used by the independent source-camera audit.
    x0,y0,w,h=0,150,120,93
    complete=rgb[y0:y0+h,:w].copy();complete[:,:,3]=union[y0:y0+h,:w]*255
    Image.fromarray(complete).save(DIRECTORY/'complete-source.png')
    write_json(DIRECTORY/'partition.json',dict(native_bbox=[x0,y0,w,h],native_mask=54,observed_domain=417,directory=str(DIRECTORY),source_sha256=sha(source)))
    packets=[]
    # Two interpenetrating lobes follow the source's visibly separate bushes.
    # Only the left lobe is completed beyond the image boundary. Mirrored local
    # leaf pixels provide an explicit provisional hidden appearance, never evidence.
    for label,left,right,known_left,known_right in [('west',-50,60,0,60),('east',40,120,60,120)]:
        folder=DIRECTORY/label;folder.mkdir();width=right-left
        image=np.zeros((h,width,4),np.uint8);observed=np.zeros_like(image)
        for local_x,world_x in enumerate(range(left,right)):
            sample=world_x if world_x>=0 else -world_x-1
            image[:,local_x]=complete[:,sample]
            if world_x<0:
                # Taper the inferred silhouette away from the map while keeping
                # every observed in-map alpha/RGB byte unchanged.
                yy=np.arange(h);gate=((world_x/54)**2+((yy-h*.53)/(h*.57))**2)<1
                image[:,local_x,3]*=gate.astype(np.uint8)
            elif known_left<=world_x<known_right:observed[:,local_x]=complete[:,sample]
        if not np.array_equal(observed[:,known_left-left:known_right-left],complete[:,known_left:known_right]):
            raise ValueError('Native observed RGB/alpha changed while partitioning bank')
        Image.fromarray(image).save(folder/'complete-source.png');Image.fromarray(observed).save(folder/'observed-source.png')
        packet=dict(directory=str(folder),native_bbox=[left,y0,width,h],native_mask=54,observed_domain=417,source_sha256=sha(source),inferred_map_edge_completion=label=='west',inferred_front_image='complete-source.png',cluster_count=450,leaf_size_range=[3.5,6.],curved_front=True,irregular_source_fragments=True,irregular_inferred_alpha=True)
        write_json(folder/'partition.json',packet);packets.append(packet)
    observed_count=sum(int((np.asarray(Image.open(Path(p['directory'])/'observed-source.png'))[:,:,3]>127).sum()) for p in packets)
    if observed_count!=int(union.sum()):raise ValueError('Observed bank partition loses or duplicates source pixels')
    write_json(DIRECTORY/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,observed_pixels=observed_count,native_mask=54,duplicate_observed_pixels=0,off_map_pixels_are_inferred=True))
    bpy.ops.wm.open_mainfile(filepath=str(previous/'input.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];objects=[];reports=[]
    for label,packet in zip(('west','east'),packets):
        obj=bpy.data.objects.new('Northwest Boundary Shrub54 '+label,bpy.data.meshes.new('Northwest Boundary Shrub54 '+label));collection.objects.link(obj)
        for key,value in dict(source_node=NODE,asset_group=ASSET,asset_name='Northwest Boundary Shrub54',part_name='Northwest foreground shrub lobes').items():obj[key]=value
        reports.append(build(obj,packet));objects.append(obj)
    visibility={o:o.hide_render for o in collection.all_objects if o.type=='MESH'}
    for o in visibility:o.hide_render=False
    inventory(DIRECTORY/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source,patch_manifest=OUT/'source-states/layers.json')
    for o,value in visibility.items():o.hide_render=value
    validate_catalog(DIRECTORY/'inventory/inventory.json',DIRECTORY/'catalog.json')
    write_json(DIRECTORY/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DIRECTORY/'catalog.json'),inventory_sha256=sha(DIRECTORY/'inventory/inventory.json'),evidence='Native54 source cutout is foreground foliage over northwest rocks50/51. Two physical lobes share one authored owner; observed pixels partition atx60 without duplicates. Western completion is inferred.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DIRECTORY/'input.blend'))
    worker=OUT/'understory-round-6/assets'/ASSET
    prepare(worker,asset_id=ASSET,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=DIRECTORY/'catalog.json',inventory_path=DIRECTORY/'inventory/inventory.json',review_path=DIRECTORY/'grouping-review.json',source_mask_manifest=DIRECTORY/'source-masks.json',width=384,height=384,framing_padding=1.25)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    write_json(inspection/'refinement.json',dict(asset_id=ASSET,mask=54,crown=dict(geometry_version='native-shrub-leaf-volume-v2',lobes=reports),source_packet=str(DIRECTORY/'partition.json'),model_sha256=sha(worker/'model.blend'),status='isolated candidate; self-review and rock joint review pending',limitations=['Western off-map silhouette and volume are inferred.','Two-lobe grouping is an editable hypothesis; no individual plant identities inferred.','Visible woody twigs are represented by source patches; volumetric branch support needs review.','No existing approved geometry changed; catalog integration and user approval pending.']))
    audit(worker);render_workspace(worker,384,release_slot=False)
    print('NORTHWEST54 SHRUB CANDIDATE',worker,flush=True)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
