"""Freeze a new worker for the corrected visible-kindling source domain."""
import json
import sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from refinement_workspace import prepare
from evidence_io import write_json
from render_slots import acquire,release


def main():
    asset='croisement02-north-kindling-bundle';old=OUT/'scenery-round-1/assets'/asset;w=OUT/'scenery-round-2/assets'/asset
    if (w/'workspace.json').exists():return
    d=OUT/'feedback-source-domains';d.mkdir(exist_ok=True)
    m=json.loads((old/'mask-reference/assignments.json').read_text());inv=json.loads(Path(m['mask_inventory']).read_text());domain=d/'visible-kindling.png';im=Image.new('L',(1792,1152));ImageDraw.Draw(im).polygon([(1593,234),(1598,214),(1603,202),(1608,198),(1614,202),(1619,211),(1629,236),(1619,240),(1604,238)],fill=255);im.save(domain)
    inv['masks'].append(dict(index=300,layer=0,png=str(domain),box_top_left=[0,0],box_size=[1792,1152],provenance='Reviewed source-visible sticks in front of the trunk; authored domain.'))
    write_json(d/'inventory.json',inv);m['mask_inventory']=str(d/'inventory.json')
    for a in m['projections']['exterior']['assignments']:
        if a.get('asset_group')==asset:a['mask_indices']=[300]
    write_json(d/'assignments.json',m)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(w,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=OUT/'scenery-domains/grouping-review.json',source_mask_manifest=d/'assignments.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    (w/'inspection').mkdir(exist_ok=True);r=json.loads((old/'inspection/refinement.json').read_text());write_json(w/'inspection/refinement.json',r);release()

if __name__=='__main__':main()
