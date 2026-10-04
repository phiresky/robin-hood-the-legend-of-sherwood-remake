"""Prepare leaf-only complements of the reviewed western rock source split."""
import json,sys
from pathlib import Path
import bpy,numpy as np
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
BASE=OUT/'ground-plant-integration'
DEST=OUT/'understory-candidates/west-complements-v1'
SPECS=((57,351,481),(60,352,482))

def main():
    catalog=json.loads(reviewed_catalog().read_text())
    if catalog!=json.loads((BASE/'catalog.json').read_text()):raise ValueError('Fresh complete source base must match current canonical catalog')
    DEST.mkdir(exist_ok=False);write_json(DEST/'previous-catalog.json',catalog)
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    manifest=json.loads((BASE/'source-masks.json').read_text());invpath=Path(manifest['mask_inventory']);masks=json.loads(invpath.read_text());rows={r['index']:r for r in masks['masks']}
    def canvas(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(invpath.parent/row['png']).convert('L'))>127;return a
    prior=np.zeros((1152,1792),bool)
    for i in (410,411,412,413):prior|=canvas(i)
    packets={};validation=[]
    for native,split,domain in SPECS:
        split_path=OUT/f'west-rock-source-revision/domain-{split}.png';observed=(np.asarray(Image.open(split_path).convert('L'))>127)&~prior
        if np.any(observed&~canvas(native)):raise ValueError('Complement escaped native source')
        path=DEST/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        node=f'foliage-shrub-{native:03}';asset=f'croisement02-shrub-{native:02}';name=f'West Rock Foliage {native}'
        catalog['groups'].append(dict(id=asset,name=name,authored_scenery=True,native_foliage_mask=native,parts=[dict(node=node,name=name,foliage_domain_mask=domain)]));catalog['canonical_owners'][node]=asset
        masks['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance=f'Reviewed rock complement{split} minus already-owned foliage410–413'))
        manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        folder=DEST/f'shrub-{native}';folder.mkdir();row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];extension=40 if x==0 else 0
        complete=np.zeros((h,w+extension,4),np.uint8);known=complete.copy()
        for col in range(w+extension):
            sx=col-extension;sample=sx if sx>=0 else -sx-1
            complete[:,col]=rgb[y:y+h,x+sample];complete[:,col,3]=observed[y:y+h,x+sample]*255
            if sx>=0:known[:,col]=complete[:,col]
            else:
                yy=np.arange(h);gate=(sx/44)**2+((yy-h*.5)/(h*.58))**2<1
                complete[:,col,3]*=gate.astype(np.uint8)
        Image.fromarray(known).save(folder/'observed-source.png');Image.fromarray(complete).save(folder/'complete-source.png')
        accepted=known[:,:,3]>127
        if int(accepted.sum())!=int(observed.sum()):raise ValueError('Leaf source partition lost pixels')
        if not np.array_equal(known[:,extension:,:3][observed[y:y+h,x:x+w]],rgb[y:y+h,x:x+w,:3][observed[y:y+h,x:x+w]]):raise ValueError('Leaf source RGB changed')
        packet=dict(directory=str(folder),native_bbox=[x-extension,y,w+extension,h],native_mask=native,observed_domain=domain,source_sha256=sha(source),inferred_front_image='complete-source.png',cluster_count=700,leaf_size_range=[3.5,6.],curved_front=True,irregular_source_fragments=True,irregular_inferred_alpha=True,source_split_path=str(split_path),source_split_sha256=sha(split_path),excluded_existing_foliage=[410,411,412,413],inferred_map_edge_completion=bool(extension),ownership_note='Physical front follows reviewed foliage complement, never excluded rock RGB. Hidden and off-map leaf appearance is inferred from this same plant.')
        write_json(folder/'partition.json',packet);packets[native]=packet
        validation.append(dict(native_mask=native,domain=domain,known_rgb_changed=0,observed_pixels=int(observed.sum()),split_sha256=sha(split_path)))
    if np.any((np.asarray(Image.open(DEST/'domain-481.png'))>0)&(np.asarray(Image.open(DEST/'domain-482.png'))>0)):raise ValueError('Duplicate new foliage ownership')
    write_json(DEST/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,duplicate_observed_pixels=0,records=validation))
    ext=manifest['projections']['exterior'];receivers={'ground',*catalog['canonical_owners']}
    for native,split,domain in SPECS:
        node=f'foliage-shrub-{native:03}';ext.setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Inferred foliage blocks foreign source only within its reviewed native leaf complement.'))
    for rule in ext['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    ground=next(r for r in ext['assignments'] if r.get('source_node')=='ground');ground['exclude_mask_indices']+=[481,482]
    write_json(DEST/'catalog.json',catalog);write_json(DEST/'mask-inventory.json',masks);manifest['mask_inventory']=str(DEST/'mask-inventory.json');write_json(DEST/'source-masks.json',manifest)
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'));bpy.context.preferences.filepaths.save_version=0;collection=bpy.data.collections['Croisement02 Working'];reports={}
    for native,packet in packets.items():
        name=f'West Rock Foliage {native}';obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=f'foliage-shrub-{native:03}',asset_group=f'croisement02-shrub-{native:02}',asset_name=name,part_name=name).items():obj[key]=value
        reports[native]=build(obj,packet)
    visibility={o:o.hide_render for o in collection.all_objects if o.type=='MESH'}
    for o in visibility:o.hide_render=False
    inventory(DEST/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source,patch_manifest=OUT/'source-states/layers.json')
    for o,value in visibility.items():o.hide_render=value
    validate_catalog(DEST/'inventory/inventory.json',DEST/'catalog.json')
    write_json(DEST/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'catalog.json'),inventory_sha256=sha(DEST/'inventory/inventory.json'),evidence='Reviewed rock split351/352 and source complement inspection; prior foliage410–413 removed. No new trunk inferred from isolated twig pixels.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'input.blend'))
    for native,split,domain in SPECS:
        bpy.ops.wm.open_mainfile(filepath=str(DEST/'input.blend'));worker=OUT/f'understory-round-6/assets/croisement02-shrub-{native:02}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=DEST/'catalog.json',inventory_path=DEST/'inventory/inventory.json',review_path=DEST/'grouping-review.json',source_mask_manifest=DEST/'source-masks.json',width=384,height=384,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        (worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/refinement.json',dict(asset_id=worker.name,mask=native,coverage_domain_mask=domain,crown=reports[native],source_packet=str(DEST/f'shrub-{native}/partition.json'),model_sha256=sha(worker/'model.blend'),status='private mixed-source foliage candidate; self-review and rock joint pending',limitations=['Hidden volume and western continuation are inferred.','Only observed leaf complement is source evidence; excluded rock and other foliage remain with their owners.','Visible twigs have source patches, not independent inferred trunk geometry.','No approved geometry changed; no user approval or texture completion claimed.']))
        audit(worker);render_workspace(worker,384,release_slot=False)
        print('WEST COMPLEMENT',native,worker,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
