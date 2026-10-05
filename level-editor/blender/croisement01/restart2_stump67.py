"""Refine the east ivy stump with a single continuous shaft and native cap outline."""
import json
import shutil
import numpy as np
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
    if shutil.disk_usage(OUT).free<25*1024**3:raise ValueError('Disk floor25GiB')
    dest=OUT/'restart2/stump67-wood-v7';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement01 Working'];asset='croisement01-east-ivy-stump'
    obj=next(o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='building-054')
    obj['asset_group']=asset
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    cap=[(1017,529),(1023,527),(1029,529),(1035,532),(1041,536),(1045,541),(1043,545),(1037,549),(1030,550),(1022,548),(1016,544),(1012,539),(1013,534)]
    record=dict(points=[dict(x=x,y=y+40,z_top=40,z_bottom=0) for x,y in cap])
    report=stump(obj,record,54,profile=(1024,580,1.20,1.18,1.16))
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==67)
    from PIL import Image,ImageDraw,ImageChops
    alpha=Image.open(row['png']).convert('L');core=Image.new('L',alpha.size)
    # Only clearly exposed cap and left bark; the bright right ivy and low leafy mass have separate ownership.
    ImageDraw.Draw(core).polygon([(29,1),(36,0),(45,4),(53,9),(56,13),(51,15),(46,15),(45,20),(40,21),(39,26),(34,27),(33,31),(26,35),(23,32),(21,29),(23,24),(24,16),(25,9)],fill=255)
    ImageDraw.Draw(core).polygon([(42,16),(44,13),(50,12),(54,11),(58,12),(63,24),(52,32),(38,31),(36,24),(38,18)],fill=0)
    ImageDraw.Draw(core).rectangle((0,29,63,74),fill=0)
    ImageDraw.Draw(core).polygon([(22,27),(25,27),(25,35),(23,34),(21,30)],fill=255)
    wood=ImageChops.multiply(alpha,core)
    left,top=row['box_top_left'];rgb=np.asarray(Image.open(OUT/'baseline/covered.png').convert('RGB').crop((left,top,left+alpha.width,top+alpha.height))).astype(float)
    # Mixed yellow/green boundary samples are deferred with ivy instead of painting leaves onto wood.
    leafy=(rgb[:,:,1]>.85*rgb[:,:,0])&(rgb[:,:,1]>1.7*rgb[:,:,2])&(rgb[:,:,1]>95)
    owned=np.asarray(wood).copy();mixed_boundary_count=int(((owned>0)&leafy).sum());owned[leafy]=0
    wood=Image.fromarray(owned);domain=dest/'wood-domain.png';wood.save(domain)
    ImageChops.subtract(alpha,wood).save(dest/'deferred-foliage-domain.png')
    native['masks'].append(dict(row,index=267,png=str(domain)))
    masks=dest/'masks.json';masks.write_text(json.dumps(native,indent=2)+'\n')
    source=dest/'source-masks.json';source.write_text(json.dumps(dict(version=1,mask_inventory=str(masks),projections=dict(exterior=dict(state='Initial east ivy stump wood only',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[267])]))),indent=2)+'\n')
    catalog=dest/'catalog.json';groups=json.loads((OUT/'catalog.json').read_text())
    terrain={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
    keep={o for o in collection.all_objects if o.type=='MESH' and (o==obj or o.get('source_node') in terrain)}
    for other in list(bpy.data.objects):
        if other.type=='MESH' and other not in keep:bpy.data.objects.remove(other,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    nodes={o.get('source_node') for o in keep};retained=[]
    for group in groups['groups']:
        parts=[p for p in group['parts'] if (f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']) in nodes]
        if parts:retained.append(dict(group,parts=parts))
    groups['groups']=retained
    groups['canonical_owners']={f"building-{part['obstacle']:03}" if 'obstacle' in part else part['node']:group['id'] for group in groups['groups'] for part in group['parts']}
    catalog.write_text(json.dumps(groups,indent=2)+'\n')
    inventory(dest/'inventory',collection_name=collection.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    grouping=dest/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask67 and original part54 define an ivy-covered cut stump. Conservative exposed cap and left bark domain excludes right and lower ivy. Compact context retains only stump and provisional terrain; inferred full shaft continues behind the ivy.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=collection.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog,inventory_path=dest/'inventory/inventory.json',review_path=grouping,source_mask_manifest=source,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    (worker/'inspection/construction.json').write_text(json.dumps(dict(status='private candidate; actual/source/contact review required',model_sha256=sha(worker/'model.blend'),wood_domain_sha256=sha(domain),geometry=report,scope='Wood only; dense right and lower ivy is separately deferred. Original part54 gameplay reference remains authoritative; neutral hidden wood is not observed source.',lower_to_cap_radius=1.20,whole_map_duplicate=False,mixed_boundary_pixels_deferred=mixed_boundary_count),indent=2)+'\n')
    import render_candidate
    sys.argv=['render','--',str(worker)];render_candidate.main()

if __name__=='__main__':main()
