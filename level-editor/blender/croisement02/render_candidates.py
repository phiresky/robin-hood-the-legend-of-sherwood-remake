"""Render completed current candidates without reading a worker being changed."""
import argparse
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from render_tree import render_workspace
from evidence_io import sha
from render_slots import release


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--masks',nargs='*',type=int);parser.add_argument('--scenery',action='store_true');parser.add_argument('--width',type=int,default=256)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    root=OUT/('scenery-round-1' if args.scenery else 'forest-v4-round-1')/'assets'
    completed=0
    for workspace in sorted(root.iterdir()):
        path=workspace/'inspection/refinement.json'
        if not path.exists():continue
        record=json.loads(path.read_text())
        if args.scenery:
            if not (workspace/'inspection/domain-review.json').exists():continue
        else:
            if record['crown'].get('geometry_version')!='native-leaf-clusters-v5':continue
            if args.masks is not None and record['mask'] not in args.masks:continue
        model_hash=sha(workspace/'model.blend')
        if record['model_sha256']!=model_hash:raise ValueError('Worker is changing: '+workspace.name)
        evidence=workspace/'inspection/actual-materials/evidence.json'
        coverage=workspace/'inspection/source-coverage/report.json'
        if evidence.exists() and json.loads(evidence.read_text())['model_sha256']==model_hash:
            if args.scenery or (coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash):continue
        render_workspace(workspace,args.width,release_slot=False)
        completed+=1
        if completed%4==0:release()
    release()

if __name__=='__main__':main()
