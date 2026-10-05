"""Review completed shed facade beside exact existing neighbours."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import inspect_leaf_clump_joint as joint
from catalog import OUT, scenery_workspace, tree_workspace
from render_slots import acquire, release


def candidate(index):
    if index != 138:
        raise ValueError('Only the reviewed shed candidate belongs in this packet')
    return OUT/'restart2-vegetation/shed-package-v1/assets/croisement02-woodcutters-shed'


if __name__ == '__main__':
    joint.worker = candidate
    acquire()
    try:
        joint.run('restart2-shed-facade-v6', [138], False, [
            scenery_workspace('croisement02-shrub-88'),
            *[tree_workspace(i) for i in (39,40,47)]])
    finally:
        release()
