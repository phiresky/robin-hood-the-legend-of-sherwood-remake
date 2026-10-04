"""Private source and contact review for newly authored native clumps."""
import argparse
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'))
import inspect_ground_plant_joint as joint
from catalog import OUT,tree_workspace,scenery_workspace
from render_slots import acquire,release
if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('mask',type=int);parser.add_argument('--version',required=True)
    parser.add_argument('--review-suffix',default='',help='Fresh evidence directory suffix for an interrupted review; worker stays unchanged')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=args.mask
    if index not in (67,68,69,70,71,72,73,75,79,80,82,91,92):raise ValueError('Reviewed native candidates only')
    joint.worker=lambda i:OUT/f'understory-candidates/native-{i}-{args.version}/assets/croisement02-shrub-{i:02}'
    neighbours={67:[11],68:[7],69:[31,32],70:[32],71:[32],72:[34],75:[38],79:[25],80:[24],82:[26,28],91:[43,45,46]}.get(index,[])
    contexts=[tree_workspace(i) for i in neighbours]
    if index in (79,80):contexts.append(scenery_workspace('croisement02-ground-plant-116'))
    if index==75:contexts.append(scenery_workspace('croisement02-east-upright-rail-fence-95'))
    acquire()
    try:joint.run(f'native-shrub-{index}-{args.version}{args.review_suffix}',[index],include_bank=index in (67,68,73,92),context_workers=contexts)
    finally:release()
