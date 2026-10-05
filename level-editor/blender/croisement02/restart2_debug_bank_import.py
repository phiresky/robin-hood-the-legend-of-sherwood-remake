"""Read-only verify library-loaded bank world transforms before ray queries."""
import sys,json
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import write_json

def main():
    source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';manifest=json.loads((OUT/'restart2-fence/sign-neighbors-v4/manifest.json').read_text());results=[]
    for key in ['north-woodland-bank','west-rock-outcrop']:
        row=manifest['inputs'][key];path=Path(row['worker'])/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(source))
        with bpy.data.libraries.load(str(path),link=False) as (src,data):data.objects=row['objects']
        for o in data.objects:
            before=np.array(o.matrix_world).tolist();bpy.context.scene.collection.objects.link(o);bpy.context.view_layer.update();after=np.array(o.matrix_world).tolist()
            results.append(dict(asset=key,object=o.name,before=before,after=after))
    dest=OUT/'restart2-fence/shrub57-bank-import-transforms.json';write_json(dest,results);print(json.dumps(results))
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
