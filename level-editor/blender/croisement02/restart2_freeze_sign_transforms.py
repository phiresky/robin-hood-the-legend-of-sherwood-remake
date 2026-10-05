"""Freeze evaluated source-scene context transforms before private sign imports."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
    manifest=OUT/'restart2-fence/sign-neighbors-v4/manifest.json';inputs=json.loads(manifest.read_text())['inputs'];rows={}
    for key in ['north-woodland-bank','west-rock-outcrop','southwest-rock-outcrop','northwest-boundary-shrub-54','shrub-57','2','shrub-77','15','16']:
        row=inputs[key];path=Path(row['worker'])/'model.blend';assert sha(path)==row['model_sha256']
        bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();matrices={}
        for name in row['objects']:
            obj=bpy.data.objects[name];matrices[name]=dict(matrix_world=[list(r) for r in obj.matrix_world],vertices=len(obj.data.vertices),polygons=len(obj.data.polygons))
        rows[key]=dict(model_sha256=sha(path),objects=matrices)
    dest=OUT/'restart2-fence/sign-context-evaluated-transforms-v1.json';assert not dest.exists();write_json(dest,dict(source_manifest_sha256=sha(manifest),inputs=rows,method='Open each exact frozen source model as active main scene, update evaluated view layer, then read matrix_world. This precedes any library import or deparenting.'));print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
