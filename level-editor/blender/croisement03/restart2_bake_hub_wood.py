"""Bake the reviewed wood prediction privately under the shared render lease."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from bake_reviewed_asset import stage

def main():
    tree=int(sys.argv[sys.argv.index('--')+1]);assert tree in (10,11);base=ROOT/f'level-editor/work/croisement03-refinement/restart2/approved-hub-textures-v1/croisement03-tree-{tree:02}/wood-input-v1'
    experiment=base/'packet-v1/experiment'
    output=experiment/'baked-projection-v1'
    assert not output.exists()
    assert shutil.disk_usage(ROOT).free>=10*1024**3
    available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3
    review=json.loads((base/'packet-v1/generation-review.json').read_text());assert review['protected_changes']==0
    for p,d in review['files'].items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==d
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'asset/model.blend'))
        bpy.context.preferences.filepaths.save_version=0
        stage(experiment/'views.json',experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png',output,texels_per_unit=2)
    finally:
        release()

if __name__=='__main__':main()
