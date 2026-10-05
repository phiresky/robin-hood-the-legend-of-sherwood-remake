"""Refine the southwest stump wood while retaining the deferred foliage domain."""
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
    dest=OUT/'restart2/stump65-wood-v1';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement01 Working'];asset='croisement01-southwest-cut-stump'
    obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='building-055')
    obj['asset_group']=asset
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    report=stump(obj,level['sight_obstacles'][55],55,profile=(583,695,.85,.86,.94))
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==65)
    domain=OUT/'stump65-source-split-v3/wood-domain.png'
    native['masks'].append(dict(row,index=265,png=str(domain)))
    masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n')
    source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial southwest stump wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[265])]))),indent=2)+'\n')
    catalog=OUT/'catalog.json'
    inventory(dest/'inventory',collection_name=collection.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    grouping=dest/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask65 and source part55 stump wood; retained existing explicit wood domain. Foreground grass remains separate. Thick lower shaft inferred from native cap, avoiding previous narrow cone.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=collection.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog,inventory_path=dest/'inventory/inventory.json',review_path=grouping,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    (worker/'inspection/construction.json').write_text(json.dumps(dict(status='private candidate; actual/source/contact review required',model_sha256=sha(worker/'model.blend'),wood_domain_sha256=sha(domain),geometry=report,scope='Wood only; source foliage deferred',lower_to_cap_radius=.85),indent=2)+'\n')
    import render_candidate
    sys.argv=['render','--',str(worker)];render_candidate.main()

if __name__=='__main__':main()
