"""Resume packet preparation from the saved, source-traced bridge construction."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from refine_bridge import *

def main():
    root=OUT/'bridge-candidate-v2'
    worker=root/'assets'/ASSET
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(root/'bridge-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=root/'catalog.json',inventory_path=root/'inventory/inventory.json',review_path=root/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    modified(worker)
    write_json(worker/'construction.json',dict(model_sha256=sha(worker/'model.blend'),status='PRIVATE HOLD: source coverage, joints and plank refinement pending',limitations=['Deck height32 is inferred pending bank/water contact reconstruction.','Plank seams and far-end rail joints remain unfinished.','All closed members are assembled components; overlap at joints is intentional.','Pier foot and obscured southeast landing need terrain-neighbor review.']))
    release()

if __name__=='__main__':main()
