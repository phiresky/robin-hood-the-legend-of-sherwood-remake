"""Prepare isolated native grass/fern candidates; never mutate canonical ownership."""
import argparse
import json
import sys
import shutil
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from ground_plant_geometry import build
from audit_candidates import audit
from render_tree import render_workspace

PLANT_FOREGROUND={111:[112,113],114:[115],120:[121,122],121:[123]}
CHOSEN={index:PLANT_FOREGROUND.get(index,[]) for index in range(111,124)}
DIRECTORY=OUT/'ground-plant-candidates/v1'
BASE=OUT/'missing-fence-candidates/v9'
FIRST_DOMAIN=440
GEOMETRY_OPTIONS={}
INHERIT_BASE_CATALOG=True


def main():
    DIRECTORY.mkdir(parents=True,exist_ok=False)
    recipe=DIRECTORY/'recipe';recipe.mkdir()
    for name in ('prepare_ground_plants.py','ground_plant_geometry.py'):
        shutil.copyfile(Path(__file__).parent/name,recipe/name)
    write_json(DIRECTORY/'placement-provenance.json',dict(
        source=sha(OUT/'animation-references/composite-frame-0.png'),
        reserved_domain=sha(OUT/'ground-texture-preparation/pending-ground-plants.png'),
        bank_worker=str(scenery_workspace('croisement02-north-woodland-bank')),
        bank_model_sha256=sha(scenery_workspace('croisement02-north-woodland-bank')/'model.blend'),
        plateau_height=36/0.8191520442889918,
        status='Independent local contact hypothesis; full joint review required; no canonical mutation',
        shared_native_pixels=73,shared_pixel_priority=PLANT_FOREGROUND,
        foreign_overlap_notes={'116':'Foreground grass overlaps pending shrub79 by153px and80 by1px; their later domains must exclude it.',
                               '119':'Foreground fern overlaps native tree23 by23px; private source manifest excludes fern domain from that receiver.'}))
    catalog=json.loads((OUT/'fence-integration/catalog.json' if INHERIT_BASE_CATALOG else reviewed_catalog()).read_text())
    write_json(DIRECTORY/'previous-catalog.json',catalog)
    source=OUT/'animation-references/composite-frame-0.png';rgb=Image.open(source).convert('RGBA')
    mask_manifest=json.loads((OUT/'fence-integration/source-masks.json').read_text())
    source_inventory=Path(mask_manifest['mask_inventory']);native=json.loads(source_inventory.read_text())
    rows={r['index']:r for r in native['masks']}
    def canvas(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];result=np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w]=np.asarray(Image.open(source_inventory.parent/row['png']).convert('L'))>0
        return result
    packets={}
    for number,(index,exclusions) in enumerate(CHOSEN.items()):
        node=f'foliage-ground-plant-{index:03}';asset=f'croisement02-ground-plant-{index:02}';domain=FIRST_DOMAIN+number
        catalog['groups'].append(dict(id=asset,name=f'Ground plant {index:02}',authored_scenery=True,native_foliage_mask=index,
            parts=[dict(node=node,name=f'Native understory foliage {index:02}',foliage_domain_mask=domain)]))
        catalog['canonical_owners'][node]=asset
        full=canvas(index);observed=full.copy()
        for exclusion in exclusions:observed&=~canvas(exclusion)
        path=DIRECTORY/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        native['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],
            provenance=f'Reviewed native shrub{index} minus foreground{exclusions}'))
        mask_manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        packet_dir=DIRECTORY/f'plant-{index:02}';packet_dir.mkdir()
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];im=rgb.crop((x,y,x+w,y+h));im.putalpha(Image.fromarray(full[y:y+h,x:x+w].astype('uint8')*255));im.save(packet_dir/'complete-source.png')
        observed_im=rgb.crop((x,y,x+w,y+h));observed_im.putalpha(Image.fromarray(observed[y:y+h,x:x+w].astype('uint8')*255));observed_im.save(packet_dir/'observed-source.png')
        packet=dict(directory=str(packet_dir),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=index,
            observed_domain=domain,excluded_foreground=exclusions,observed_pixels=int(observed.sum()),
            complete_native_pixels=int(full.sum()),source_sha256=sha(source),
            ownership_note='Complete physical clump follows native silhouette; covered regions have inferred appearance only. Source evidence excludes the explicitly listed overlapping foreground masks.',ground_z=(36/0.8191520442889918 if index>=117 else 0),**GEOMETRY_OPTIONS)
        write_json(packet_dir/'partition.json',packet);packets[index]=packet
    sheet=Image.new('RGB',(512*len(CHOSEN),1000),'#888888');draw=ImageDraw.Draw(sheet);validation=[]
    source_rgb=np.asarray(rgb)
    for column,index in enumerate(CHOSEN):
        packet=packets[index];x,y,w,h=packet['native_bbox']
        observed=np.asarray(Image.open(DIRECTORY/f'plant-{index:02}/observed-source.png').convert('RGBA'))
        accepted=observed[:,:,3]>127
        if not np.array_equal(observed[:,:,:3][accepted],source_rgb[y:y+h,x:x+w,:3][accepted]):raise ValueError('Observed RGB changed')
        validation.append(dict(native_mask=index,accepted_pixels=int(accepted.sum()),rejected_foreground_pixels=packet['complete_native_pixels']-int(accepted.sum()),known_rgb_changed=0))
        for row,name in enumerate(('complete-source.png','observed-source.png')):
            im=Image.open(DIRECTORY/f'plant-{index:02}'/name).convert('RGBA')
            im=im.resize((im.width*3,im.height*3),Image.Resampling.NEAREST)
            sheet.paste(im,(column*512,row*480+35),im);draw.text((column*512+4,row*480+5),f'{index} {name}',fill='black')
    sheet.save(DIRECTORY/'source-ownership-sheet.png')
    domains=[np.asarray(Image.open(DIRECTORY/f'domain-{FIRST_DOMAIN+i}.png').convert('L'))>0 for i in range(len(CHOSEN))]
    duplicate=sum(int((a&b).sum()) for i,a in enumerate(domains) for b in domains[i+1:])
    if duplicate:raise ValueError('Duplicate observed ownership across new shrubs')
    write_json(DIRECTORY/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,duplicate_observed_pixels=0,records=validation))
    receivers={'ground',*catalog['canonical_owners']}
    for number,index in enumerate(CHOSEN):
        node=f'foliage-ground-plant-{index:03}';domain=FIRST_DOMAIN+number
        mask_manifest['projections']['exterior'].setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,
            receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Hidden clump volume cannot block foreign source receivers outside its own observed leaf domain.'))
    for rule in mask_manifest['projections']['exterior']['occluder_constraints']:
        if rule['source_node'].startswith('foliage-'):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    ground=next(a for a in mask_manifest['projections']['exterior']['assignments'] if a.get('source_node')=='ground')
    ground['exclude_mask_indices']+=list(range(FIRST_DOMAIN,FIRST_DOMAIN+len(CHOSEN)))
    ground['exclusion_reason']+=' Authored shrub '+','.join(map(str,CHOSEN))+' observed leaf domains also leave the ground receiver.'
    if 119 in CHOSEN:
        for assignment in mask_manifest['projections']['exterior']['assignments']:
            if assignment.get('asset_group')=='croisement02-tree-23' and 23 in assignment.get('mask_indices',[]):
                assignment.setdefault('exclude_mask_indices',[]).append(448)
                assignment.update(exclusions_reviewed=True,exclusion_reason='Native fern119 visibly crosses tree23 base; its23 pixels belong to foreground plant.')
    write_json(DIRECTORY/'catalog.json',catalog);write_json(DIRECTORY/'mask-inventory.json',native)
    mask_manifest['mask_inventory']=str(DIRECTORY/'mask-inventory.json');write_json(DIRECTORY/'source-masks.json',mask_manifest)
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];reports={}
    present={o.get('source_node') for o in collection.all_objects if o.type=='MESH'}
    added_context=[]
    for node,asset in catalog['canonical_owners'].items():
        if node in present or node.startswith('foliage-ground-plant-'):continue
        worker=scenery_workspace(asset)
        rows=json.loads((OUT/'fence-integration/inventory.json').read_text())['objects']
        names={r['object'] for r in rows if r.get('source_node')==node}
        with bpy.data.libraries.load(str(worker/'model.blend'),link=False) as (src,dst):
            if not names<=set(src.objects):raise ValueError(f'Context names missing: {names-set(src.objects)}')
            dst.objects=sorted(names)
        selected=[]
        for obj in dst.objects:
            if obj is not None:
                collection.objects.link(obj)
                parent=obj.parent
                while parent:
                    if not parent.users_collection:collection.objects.link(parent)
                    parent=parent.parent
        bpy.context.view_layer.update()
        for obj in dst.objects:
            if obj is not None and obj.type=='MESH' and obj.get('source_node')==node:
                world=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=world;selected.append(obj)
        for obj in dst.objects:
            if obj is not None and obj not in selected:bpy.data.objects.remove(obj,do_unlink=True)
        if not selected:raise ValueError(f'Missing context {node} in {worker}')
        added_context.append(dict(node=node,worker=str(worker),model_sha256=sha(worker/'model.blend')))
    write_json(DIRECTORY/'context-inputs.json',added_context)
    for index,packet in packets.items():
        asset=f'croisement02-ground-plant-{index:02}';name=f'Ground plant {index:02}'
        obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=f'foliage-ground-plant-{index:03}',asset_group=asset,asset_name=name,part_name=name).items():obj[key]=value
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
        worker=DIRECTORY/f'assets/croisement02-ground-plant-{index:02}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,
            grouping_manifest=DIRECTORY/'catalog.json',inventory_path=DIRECTORY/'inventory/inventory.json',review_path=DIRECTORY/'grouping-review.json',
            source_mask_manifest=DIRECTORY/'source-masks.json',width=256,height=256,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
        write_json(inspection/'refinement.json',dict(asset_id=worker.name,mask=index,crown=reports[index],source_packet=str(DIRECTORY/f'plant-{index:02}/partition.json'),
            model_sha256=sha(worker/'model.blend'),status='isolated authored ground-plant candidate; visual review pending',limitations=[
                'Native plant silhouette is observed; hidden blades and rear leaf arrangement are inferred.',
                'Shared plant pixels use explicit later-native-mask priority; 13 native masks retain their complete physical silhouettes. Rear colors reuse only this plant visible palette.',
                'Native artwork only; no tree references or generated textures.',
                'No native sight obstacle is fabricated. Full terrain refresh, joint placement and user geometry approval remain pending.']))
        audit(worker);render_workspace(worker,256,release_slot=False)
        coverage=json.loads((inspection/'source-coverage/report.json').read_text())
        if coverage['intersection_over_union']<.95:
            raise ValueError(f'Plant{index} source coverage failed: {coverage}')
        write_json(inspection/'candidate-state.json',dict(
            model_sha256=sha(worker/'model.blend'),technical_checks='PASS',
            user_approval=None,texture_generation='not requested; native-only inferred backs',
            status='private; eight-view visual and joint terrain review still required'))
        print('GROUND PLANT CANDIDATE',index,flush=True)
    write_json(DIRECTORY/'candidates.json',dict(status='isolated geometry candidates; not integrated or user-approved',masks=list(CHOSEN),catalog_sha256=sha(DIRECTORY/'catalog.json')))


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--masks',nargs='+',type=int,default=list(range(111,124)))
    parser.add_argument('--version',default='v1')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    if not set(args.masks)<=set(range(111,124)):raise ValueError('Only native ground plants111–123')
    # Domain IDs follow native IDs even for an isolated preview subset.
    if args.masks!=list(range(min(args.masks),max(args.masks)+1)):raise ValueError('Use a contiguous subset')
    CHOSEN={i:PLANT_FOREGROUND.get(i,[]) for i in args.masks};FIRST_DOMAIN=440+min(args.masks)-111
    DIRECTORY=OUT/'ground-plant-candidates'/args.version
    acquire()
    try:main()
    finally:release()
