"""Review boundary-completed shrub93 beside exact existing neighbours."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import inspect_leaf_clump_joint as joint
from catalog import OUT, scenery_workspace
from render_slots import acquire, release


def candidate(index):
    if index != 93:
        raise ValueError('Only the reviewed boundary candidate93 belongs in this packet')
    return OUT/'restart2-vegetation/shrub93-package-v1/assets/croisement02-shrub-93'


if __name__ == '__main__':
    joint.worker = candidate
    acquire()
    try:
        joint.run('restart2-shrub93-boundary-v1', [93], False, [
            OUT/'tree35-root-research/candidate-v14',
            scenery_workspace('croisement02-shrub-85'),
            scenery_workspace('croisement02-east-upright-rail-fence-95')])
    finally:
        release()
