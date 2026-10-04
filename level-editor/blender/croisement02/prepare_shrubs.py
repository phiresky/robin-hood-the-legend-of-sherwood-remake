"""Prepare isolated authored understory clumps without changing approved assets."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from shrub_geometry import build
from audit_candidates import audit
from render_tree import render_workspace

CHOSEN={55:[64,133],58:[59,60,133],59:[60,133]}
DIRECTORY=OUT/'understory-candidates/clumps-v1'
BASE=OUT/'authored-stem-integration'
FIRST_DOMAIN=410
GEOMETRY_OPTIONS={}


def main():
    DIRECTORY.mkdir(exist_ok=False)
    catalog=json.loads(reviewed_catalog().read_text())
    write_json(DIRECTORY/'previous-catalog.json',catalog)
    source=OUT/'animation-references/composite-frame-0.png';rgb=Image.open(source).convert('RGBA')
    mask_manifest=json.loads((BASE/'source-masks.json').read_text())
    source_inventory=Path(mask_manifest['mask_inventory']);native=json.loads(source_inventory.read_text())
    rows={r['index']:r for r in native['masks']}
    def canvas(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];result=np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w]=np.asarray(Image.open(source_inventory.parent/row['png']).convert('L'))>0
        return result
    packets={}
    for number,(index,exclusions) in enumerate(CHOSEN.items()):
        node=f'foliage-shrub-{index:03}';asset=f'croisement02-shrub-{index:02}';domain=FIRST_DOMAIN+number
        catalog['groups'].append(dict(id=asset,name=f'Understory Shrub {index:02}',authored_scenery=True,native_foliage_mask=index,
            parts=[dict(node=node,name=f'Native understory foliage {index:02}',foliage_domain_mask=domain)]))
        catalog['canonical_owners'][node]=asset
        full=canvas(index);observed=full.copy()
        for exclusion in exclusions:observed&=~canvas(exclusion)
        path=DIRECTORY/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        native['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],
            provenance=f'Reviewed native shrub{index} minus foreground{exclusions}'))
        mask_manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        packet_dir=DIRECTORY/f'shrub-{index:02}';packet_dir.mkdir()
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];im=rgb.crop((x,y,x+w,y+h));im.putalpha(Image.fromarray(full[y:y+h,x:x+w].astype('uint8')*255));im.save(packet_dir/'complete-source.png')
        observed_im=rgb.crop((x,y,x+w,y+h));observed_im.putalpha(Image.fromarray(observed[y:y+h,x:x+w].astype('uint8')*255));observed_im.save(packet_dir/'observed-source.png')
        packet=dict(directory=str(packet_dir),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=index,
            observed_domain=domain,excluded_foreground=exclusions,observed_pixels=int(observed.sum()),
            complete_native_pixels=int(full.sum()),source_sha256=sha(source),
            ownership_note='Complete physical clump follows native silhouette; covered regions have inferred appearance only. Source evidence excludes the explicitly listed overlapping foreground masks.',**GEOMETRY_OPTIONS)
        write_json(packet_dir/'partition.json',packet);packets[index]=packet
    sheet=Image.new('RGB',(512*len(CHOSEN),1000),'#888888');draw=ImageDraw.Draw(sheet);validation=[]
    source_rgb=np.asarray(rgb)
    for column,index in enumerate(CHOSEN):
        packet=packets[index];x,y,w,h=packet['native_bbox']
        observed=np.asarray(Image.open(DIRECTORY/f'shrub-{index:02}/observed-source.png').convert('RGBA'))
        accepted=observed[:,:,3]>127
        if not np.array_equal(observed[:,:,:3][accepted],source_rgb[y:y+h,x:x+w,:3][accepted]):raise ValueError('Observed RGB changed')
        validation.append(dict(native_mask=index,accepted_pixels=int(accepted.sum()),rejected_foreground_pixels=packet['complete_native_pixels']-int(accepted.sum()),known_rgb_changed=0))
        for row,name in enumerate(('complete-source.png','observed-source.png')):
            im=Image.open(DIRECTORY/f'shrub-{index:02}'/name).convert('RGBA')
            im=im.resize((im.width*3,im.height*3),Image.Resampling.NEAREST)
            sheet.paste(im,(column*512,row*480+35),im);draw.text((column*512+4,row*480+5),f'{index} {name}',fill='black')
    sheet.save(DIRECTORY/'source-ownership-sheet.png')
    domains=[np.asarray(Image.open(DIRECTORY/f'domain-{FIRST_DOMAIN+i}.png').convert('L'))>0 for i in range(len(CHOSEN))]
    duplicate=sum(int((a&b).sum()) for i,a in enumerate(domains) for b in domains[i+1:])
    if duplicate:raise ValueError('Duplicate observed ownership across new shrubs')
    write_json(DIRECTORY/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,duplicate_observed_pixels=0,records=validation))
    receivers={'ground',*catalog['canonical_owners']}
    for number,index in enumerate(CHOSEN):
        node=f'foliage-shrub-{index:03}';domain=FIRST_DOMAIN+number
        mask_manifest['projections']['exterior'].setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,
            receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Hidden clump volume cannot block foreign source receivers outside its own observed leaf domain.'))
    for rule in mask_manifest['projections']['exterior']['occluder_constraints']:
        if rule['source_node'].startswith('foliage-'):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    ground=next(a for a in mask_manifest['projections']['exterior']['assignments'] if a.get('source_node')=='ground')
    ground['exclude_mask_indices']+=list(range(FIRST_DOMAIN,FIRST_DOMAIN+len(CHOSEN)))
    ground['exclusion_reason']+=' Authored shrub '+','.join(map(str,CHOSEN))+' observed leaf domains also leave the ground receiver.'
    write_json(DIRECTORY/'catalog.json',catalog);write_json(DIRECTORY/'mask-inventory.json',native)
    mask_manifest['mask_inventory']=str(DIRECTORY/'mask-inventory.json');write_json(DIRECTORY/'source-masks.json',mask_manifest)
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];reports={}
    for index,packet in packets.items():
        asset=f'croisement02-shrub-{index:02}';name=f'Understory Shrub {index:02}'
        obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=f'foliage-shrub-{index:03}',asset_group=asset,asset_name=name,part_name=name).items():obj[key]=value
        reports[index]=build(obj,packet)
    visibility={o:o.hide_render for o in collection.all_objects if o.type=='MESH'}
    for o in visibility:o.hide_render=False
    inventory(DIRECTORY/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source,patch_manifest=OUT/'source-states/layers.json')
    for o,value in visibility.items():o.hide_render=value
    validate_catalog(DIRECTORY/'inventory/inventory.json',DIRECTORY/'catalog.json')
    write_json(DIRECTORY/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DIRECTORY/'catalog.json'),
        inventory_sha256=sha(DIRECTORY/'inventory/inventory.json'),evidence='Source cutouts and context '+','.join(map(str,CHOSEN))+' are leafy clumps. Explicit foreground exclusions are recorded per source packet; existing sources unchanged.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DIRECTORY/'input.blend'))
    for index in CHOSEN:
        bpy.ops.wm.open_mainfile(filepath=str(DIRECTORY/'input.blend'))
        worker=OUT/f'understory-round-1/assets/croisement02-shrub-{index:02}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,
            grouping_manifest=DIRECTORY/'catalog.json',inventory_path=DIRECTORY/'inventory/inventory.json',review_path=DIRECTORY/'grouping-review.json',
            source_mask_manifest=DIRECTORY/'source-masks.json',width=384,height=384,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
        write_json(inspection/'refinement.json',dict(asset_id=worker.name,mask=index,crown=reports[index],source_packet=str(DIRECTORY/f'shrub-{index:02}/partition.json'),
            model_sha256=sha(worker/'model.blend'),status='isolated authored shrub candidate; visual review pending',limitations=[
                'Original shrub silhouette is observed; round hidden depth and rear leaf arrangement are inferred.',
                'Known source leaves exclude overlapping foreground masks; covered/front and rear colors reuse only this shrub own visible leaf palette.',
                'No tree asset was borrowed beyond the two permitted construction references.',
                'No native sight obstacle is fabricated. Full terrain refresh, joint placement and user geometry approval remain pending.']))
        audit(worker);render_workspace(worker,384,release_slot=False)
        print('SHRUB CANDIDATE',index,flush=True)
    write_json(DIRECTORY/'candidates.json',dict(status='isolated geometry candidates; not integrated or user-approved',masks=list(CHOSEN),catalog_sha256=sha(DIRECTORY/'catalog.json')))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--batch',choices=['initial','southwest81'],default='initial')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    if args.batch=='southwest81':
        CHOSEN={81:[]};DIRECTORY=OUT/'understory-candidates/southwest81-v1'
        BASE=OUT/'understory-candidates/west-bank-v4';FIRST_DOMAIN=414;GEOMETRY_OPTIONS={'curved_front':True}
    acquire()
    try:main()
    finally:release()
