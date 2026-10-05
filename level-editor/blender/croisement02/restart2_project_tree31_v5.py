"""Project the continuous lower31 volume with unchanged native ownership."""
import json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from refinement_workspace import prepare,modified
from render_slots import acquire,release

def main():
 old=tree_workspace(31);prototype=OUT/'restart2-wood/tree31-sdf-v5';worker=OUT/'restart2-wood/projected31-v5/assets'/old.name;cfg=json.loads((old/'workspace.json').read_text())
 if worker.exists():raise FileExistsError(worker)
 bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=cfg['framing_padding'],lighting=cfg['lighting'])
 bpy.ops.wm.open_mainfile(filepath=str(prototype/'model.blend'));bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
 import restart2_restore_tree31_v5
 sys.argv=[sys.argv[0],'--','31'];restart2_restore_tree31_v5.main()
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
