"""Apply visually selected same-tree bark donors to unobserved texels only."""
import argparse
import json
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from bark_materials import fill
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import validate
from audit_candidates import audit
from render_tree import render_workspace


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--masks',nargs='+',type=int,required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    decisions={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    try:
        for mask in args.masks:
            w=OUT/f'forest-v4-round-1/assets/croisement02-tree-{mask:02}'
            if decisions.get(w.name,{}).get('decision')=='approved':raise ValueError('Approved worker must remain frozen')
            selection=w/'inspection/bark-donor-selection.json'
            if not selection.exists():raise ValueError('Missing reviewed donor selection')
            receipt=w/'inspection/bark-revision.json';model=w/'model.blend'
            if receipt.exists():
                r=json.loads(receipt.read_text())
                if r['model_sha256']!=sha(model) or r['selection_sha256']!=sha(selection):raise ValueError('Bark revision changed')
            else:
                acquire();bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.preferences.filepaths.save_version=0;validate(w)
                before=sha(model);objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name]
                from revise_feedback import geometry
                geometry_hash=geometry(objects);result=fill(w,objects,mask)
                if geometry(objects)!=geometry_hash:raise ValueError('Bark fill changed geometry')
                validate(w);bpy.ops.wm.save_as_mainfile(filepath=str(model))
                report=json.loads((w/'inspection/refinement.json').read_text());report['bark']=result;report['model_sha256']=sha(model)
                write_json(w/'inspection/refinement.json',report);audit(w)
                write_json(receipt,dict(before_model_sha256=before,model_sha256=sha(model),selection_sha256=sha(selection),geometry_sha256=geometry_hash,geometry_unchanged=True,known_rgb_unchanged=True))
            evidence=w/'inspection/actual-materials/evidence.json';coverage=w/'inspection/source-coverage/report.json'
            if not all(p.exists() and json.loads(p.read_text())['model_sha256']==sha(model) for p in (evidence,coverage)):
                render_workspace(w,256,release_slot=False)
            release();print('BARK REVIEW',mask,flush=True)
    finally:release()


if __name__=='__main__':main()
