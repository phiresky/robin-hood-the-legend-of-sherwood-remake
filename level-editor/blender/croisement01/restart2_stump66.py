"""Refine the south cut stump with a single continuous shaft and native cap outline."""
import json
import sys
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).parent))
from prepare_props import stump, OUT
from refinement_workspace import prepare, modified, validate
from refinement_inventory import inventory
from render_slots import acquire
from evidence_io import sha

def main():
    dest=OUT/'restart2/stump66-wood-v1';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement01 Working'];asset='croisement01-south-cut-stump'
    obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='building-056')
    obj['asset_group']=asset
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    cap=[(855,734),(863,734),(870,736),(875,740),(878,745),(876,751),(870,756),(862,759),(853,758),(846,754),(842,749),(842,744),(846,739),(850,736)]
    record=dict(points=[dict(x=x,y=y+24,z_top=24,z_bottom=0) for x,y in cap])
    report=stump(obj,record,56,profile=(858,778,.85,.9,.96))
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==66)
    from PIL import Image,ImageDraw,ImageChops
    alpha=Image.open(row['png']).convert('L');core=Image.new('L',alpha.size)
    ImageDraw.Draw(core).polygon([(31,0),(40,0),(49,4),(54,10),(55,19),(52,30),(49,38),(41,43),(31,44),(24,40),(19,34),(17,27),(17,13),(23,5)],fill=255)
    wood=ImageChops.multiply(alpha,core);domain=dest/'wood-domain.png';wood.save(domain)
    ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
    native['masks'].append(dict(row,index=266,png=str(domain)))
    masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n')
    source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial south stump wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[266])]))),indent=2)+'\n')
    catalog=dest/'catalog.json';groups=json.loads((OUT/'catalog.json').read_text())
    for group in groups['groups']:
        if group['id']==asset:group['parts']=[p for p in group['parts'] if p.get('obstacle')==56]
    groups['groups'].append(dict(id='croisement01-south-stump-retired-cap-reference',name='Unchanged original cap reference',parts=[dict(obstacle=67,name='Original cap reference only')]))
    groups['canonical_owners']={f"building-{part['obstacle']:03}" if 'obstacle' in part else part['node']:group['id'] for group in groups['groups'] for part in group['parts']}
    catalog.write_text(json.dumps(groups,indent=2)+'\n')
    previous=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='building-067');previous['asset_group']='croisement01-south-stump-retired-cap-reference'
    inventory(dest/'inventory',collection_name=collection.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    grouping=dest/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask66 and original parts56/67 define a single cut stump. One closed continuous shaft includes the measured cap; former cap remains an unchanged reference outside this worker. Conservative wood ownership excludes foreground grass.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=collection.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog,inventory_path=dest/'inventory/inventory.json',review_path=grouping,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    (worker/'inspection/construction.json').write_text(json.dumps(dict(status='private candidate; actual/source/contact review required',model_sha256=sha(worker/'model.blend'),wood_domain_sha256=sha(domain),geometry=report,scope='Wood only; native foliage deferred. Original56/67 cap and body consolidated; integration must preserve both gameplay references.',lower_to_cap_radius=.85),indent=2)+'\n')
    import render_candidate
    sys.argv=['render','--',str(worker)];render_candidate.main()

if __name__=='__main__':main()
