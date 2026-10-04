"""Compare private flowering clumps with the exact existing wattle worker."""
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from catalog import OUT,scenery_workspace
from render_slots import acquire,release
import inspect_ground_plant_joint as joint


def main():
    candidate=OUT/'understory-candidates/native76-clumps-v1/assets/croisement02-shrub-76'
    joint.worker=lambda index:candidate if index==76 else (_ for _ in ()).throw(ValueError('Unexpected plant'))
    joint.run('wattle-flowers76-v1',[76],include_bank=False,context_workers=[scenery_workspace('croisement02-southwest-path-wattle-fence')])

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
