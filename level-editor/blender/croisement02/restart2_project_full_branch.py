"""Project the complete continuous branch volume with unchanged native ownership."""
import argparse,json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from refinement_workspace import prepare,modified
from render_slots import acquire,release

def main():
 parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[43,45,46]);index=parser.parse_args(sys.argv[sys.argv.index('--')+1:]).index
 old=tree_workspace(index);prototype=OUT/f'restart2-wood/tree{index}-branch-fitted-v{3 if index==46 else 2}';worker=OUT/f'restart2-wood/branch-projected-v{4 if index==46 else 3}/assets'/old.name;cfg=json.loads((old/'workspace.json').read_text())
 if worker.exists():raise FileExistsError(worker)
 bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=cfg['framing_padding'],lighting=cfg['lighting'])
 bpy.ops.wm.open_mainfile(filepath=str(prototype/'model.blend'));bpy.context.preferences.filepaths.save_version=0
 # The reconstructed surfaces have a uniform neutral fallback, not an inherited atlas.
 for obj in bpy.data.collections['Croisement02 Working'].all_objects:
  if obj.type=='MESH' and obj.get('asset_group')==old.name and obj.get('projection_component')!='crown' and not obj.data.uv_layers:
   if not obj.data.materials:raise ValueError('Missing explicit neutral material')
   obj.data.uv_layers.new(name='NeutralFallback')
 bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
 import restart2_restore_full_branch
 sys.argv=[sys.argv[0],'--',str(index)];restart2_restore_full_branch.main()
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
