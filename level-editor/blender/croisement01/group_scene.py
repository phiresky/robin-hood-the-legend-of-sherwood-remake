"""Create a grouped candidate scene while preserving the frozen native baseline."""
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from group_assets import group_assets
from render_slots import acquire

def main():
    destination=OUT/'croisement01-grouped.blend'
    if destination.exists():raise FileExistsError(destination)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'baseline/croisement01-baseline.blend'))
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(destination));group_assets(OUT/'catalog.json')
    from refinement_inventory import inventory
    inventory(OUT/'grouped-inventory',collection_name='Croisement01 Working',map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')

if __name__=='__main__':main()
