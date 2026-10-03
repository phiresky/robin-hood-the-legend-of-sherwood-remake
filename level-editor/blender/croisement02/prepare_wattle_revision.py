"""Freeze reviewed visible weave coverage missing from the actor-occlusion mask."""
import json
import sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from feedback_geometry import wattle_top,WATTLE_POSTS
from refinement_workspace import prepare
from evidence_io import write_json
from render_slots import acquire,release


def main():
    asset='croisement02-south-field-wattle-fence';old=OUT/'scenery-round-1/assets'/asset;w=OUT/'scenery-round-2/assets'/asset
    if (w/'workspace.json').exists():return
    d=OUT/'feedback-wattle-domains';d.mkdir(exist_ok=True);m=json.loads((old/'source-masks.json').read_text());inv=json.loads(Path(m['mask_inventory']).read_text());row=next(r for r in inv['masks'] if r['index']==98)
    im=Image.new('L',(1792,1152));im.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']));draw=ImageDraw.Draw(im)
    xs=list(range(880,1191,2));draw.polygon([(x,wattle_top(x)-2) for x in xs]+[(x,wattle_top(x)+24) for x in reversed(xs)],fill=255)
    for x,top in WATTLE_POSTS:draw.polygon([(x-3,top-1),(x+3,top-1),(x+3,wattle_top(x)+25),(x-3,wattle_top(x)+25)],fill=255)
    domain=d/'visible-weave.png';im.save(domain);inv['masks'].append(dict(index=301,layer=0,png=str(domain),box_top_left=[0,0],box_size=[1792,1152],provenance='Native mask 98 plus surveyed visible weave and posts. Actor occlusion mask has empty columns at x1000-1030 despite visible fence artwork.'))
    write_json(d/'inventory.json',inv);m['mask_inventory']=str(d/'inventory.json')
    for a in m['projections']['exterior']['assignments']:
        if a.get('asset_group')==asset:a['mask_indices']=[301]
    write_json(d/'assignments.json',m);acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(w,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=OUT/'scenery-domains/grouping-review.json',source_mask_manifest=d/'assignments.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    (w/'inspection').mkdir(exist_ok=True);write_json(w/'inspection/refinement.json',json.loads((old/'inspection/refinement.json').read_text()));release()

if __name__=='__main__':main()
