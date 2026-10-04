"""Private source and contact review for newly authored native clumps."""
import argparse
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'))
import inspect_ground_plant_joint as joint
from catalog import OUT,tree_workspace
from render_slots import acquire,release
if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('mask',type=int);parser.add_argument('--version',required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=args.mask
    if index not in (67,68,69,70,71,72,73,92):raise ValueError('Central candidates only')
    joint.worker=lambda i:OUT/f'understory-candidates/native-{i}-{args.version}/assets/croisement02-shrub-{i:02}'
    neighbours={67:[11],68:[7],69:[31,32],70:[32],71:[32],72:[34]}.get(index,[])
    acquire()
    try:joint.run(f'native-shrub-{index}-{args.version}',[index],include_bank=index in (67,68,73,92),context_workers=[tree_workspace(i) for i in neighbours])
    finally:release()
