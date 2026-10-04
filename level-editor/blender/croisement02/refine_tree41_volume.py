"""Replace unapproved tree41's regular crown cards with an irregular volume."""
import argparse
import json
import shutil
import sys
import uuid
from pathlib import Path
import bpy

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from refinement_workspace import modified, validate
from render_slots import acquire, release
from rounded_interior_geometry import build
from audit_candidates import audit
from render_tree import render_workspace
from bark_materials import fill


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--redo',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    worker=OUT/'forest-v4-round-1/assets/croisement02-tree-41'
    latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    if latest.get(worker.name,{}).get('decision')=='approved':raise ValueError('Approved geometry is frozen')
    receipt=worker/'inspection/interior-volume-revision.json'
    if receipt.exists() and not args.redo:raise ValueError('Explicit redo required')
    backup=worker.with_name(worker.name+'-volume-archive-'+uuid.uuid4().hex[:8])
    shutil.copytree(worker,backup)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        bpy.context.preferences.filepaths.save_version=0
        validate(worker)
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects
                 if o.type=='MESH' and o.get('asset_group')==worker.name]
        crown=next(o for o in objects if o.get('projection_component')=='crown')
        report=json.loads((worker/'inspection/refinement.json').read_text())
        packet=json.loads(Path(report['source_packet']).read_text())
        source=next(r for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==41)
        result=build(crown,packet,source['ground_y'])
        modified(worker)
        bark=fill(worker,objects,41,receiver_only=True,donor_mapping='continuous-grain')
        validate(worker)
        bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        report['crown']=result;report['bark']=bark;report['model_sha256']=sha(worker/'model.blend')
        report['limitations'].append('Interior crown depth uses jittered observed patches and irregular inferred leaf clusters; only the two approved Leicester construction references were used.')
        write_json(worker/'inspection/refinement.json',report)
        audit(worker)
        write_json(receipt,dict(model_sha256=report['model_sha256'],previous_model_sha256=sha(backup/'model.blend'),
            previous_worker=str(backup),algorithm='jittered-source-patches-and-irregular-world-volume-v1'))
        render_workspace(worker,384,release_slot=False)
    finally:release()


if __name__=='__main__':main()
