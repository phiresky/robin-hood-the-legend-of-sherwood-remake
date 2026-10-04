"""Rebuild unapproved clump geometry and remeasure saved native coverage."""
import json
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from shrub_geometry import build
from refinement_workspace import modified
from audit_candidates import audit
from render_tree import render_workspace

def main(indices):
    decisions={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    for index in indices:
        worker=OUT/f'understory-round-1/assets/croisement02-shrub-{index:02}'
        if decisions.get(worker.name,{}).get('decision')=='approved':raise ValueError('Approved shrub geometry is frozen')
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.preferences.filepaths.save_version=0
        report=json.loads((worker/'inspection/refinement.json').read_text());packet=json.loads(Path(report['source_packet']).read_text())
        obj=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name)
        report['crown']=build(obj,packet)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        report['model_sha256']=sha(worker/'model.blend');write_json(worker/'inspection/refinement.json',report)
        audit(worker);render_workspace(worker,384,release_slot=False)
        print('REFINED SHRUB',index,flush=True)

if __name__=='__main__':
    indices=list(map(int,sys.argv[sys.argv.index('--')+1:])) if '--' in sys.argv else [55,58,59]
    acquire()
    try:main(indices)
    finally:release()
