"""Resume a prepared zero-elevation worker using the actual grouped ground name."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import modified
from evidence_io import write_json,sha

def main():
    w=ROOT/'level-editor/work/croisement03-refinement/restart2/bridge-v6/assets/croisement03-timber-bridge'
    acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    config=json.loads((w/'workspace.json').read_text());objects=list(bpy.data.collections['Croisement03 Working'].all_objects);ground=[o for o in objects if o.type=='MESH' and o.get('source_node')=='ground'];assert len(ground)==1
    config['source_projection_ground_exclusion']['object_name']=ground[0].name;write_json(w/'workspace.json',config);modified(w)
    write_json(w/'construction.json',dict(model_sha256=sha(w/'model.blend'),status='Private geometry, joint water/bank review pending',deck_z=0,limitations=['Native zero-elevation landing frame; unknown river depth is inferred.','Actual joint land/water proof remains required before readiness.']))
    release()

if __name__=='__main__':main()
