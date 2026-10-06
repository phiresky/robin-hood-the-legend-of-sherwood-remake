"""Bake the reviewed wood prediction privately under the shared render lease."""
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from bake_reviewed_asset import stage

def main():
    tree=int(sys.argv[sys.argv.index('--')+1]);assert tree in (12,14);base=ROOT/f'level-editor/work/croisement03-refinement/restart2/tree{tree}-approved-wood-texture-v1'
    experiment=base/'packet-v1/experiment'
    output=experiment/'baked-projection-v1'
    assert not output.exists()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'asset/model.blend'))
        bpy.context.preferences.filepaths.save_version=0
        stage(experiment/'views.json',experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png',output,texels_per_unit=2)
    finally:
        release()

if __name__=='__main__':main()
