"""Continue the preserved branch experiment in an isolated restart workspace."""
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import prepare_branch


def main():
    previous = prepare_branch.OUT
    output = previous / 'restart2/branch-source-fit-v3'
    output.mkdir(parents=True, exist_ok=False)
    for name in ('baseline', 'catalog.json', 'grouped-inventory', 'croisement01-grouped.blend'):
        source = previous / name
        if not source.exists():
            raise FileNotFoundError(source)
        (output / name).symlink_to(source)
    # The prior actual native camera showed wood missing along the twig's
    # right flank, the left broken ridge, and the descending right end.
    # These small local corrections preserve the overall bent axis rather
    # than replacing it with a silhouette extrusion.
    original_tube = prepare_branch.tube

    def corrected_tube(name, trace):
        changed = []
        for x, y, z, radius in trace:
            if name == 'Upward twig union operand':
                x += .2
                radius += 1.1
                z += 1.1
            elif name == 'Continuous main branch':
                left = max(0., 1-abs(x-1048)/18)
                y -= 3.4 * left
                radius += 2.3 * left
                z += 2.3 * left
                ridge = max(0., 1-abs(x-1080)/17)
                y -= 1.4 * ridge
                right = max(0., min(1., (x-1185)/33))
                y += 1.5 * right
                radius += 3.5 * right
            elif name == 'Right source fork union operand':
                radius += .7
                z += .7
            changed.append((x, y, z, radius))
        return original_tube(name, changed)

    prepare_branch.OUT = output
    prepare_branch.tube = corrected_tube
    prior = previous/'branch-round-10/assets/croisement01-east-fallen-branch/model.blend'
    (output/'continuation.json').write_text(json.dumps(dict(
        status='private geometry candidate; no approval inherited',
        parent_model=str(prior), parent_sha256=hashlib.sha256(prior.read_bytes()).hexdigest(),
        hypothesis='Repair native wood flank misses with local tube center/radius adjustments.',
        remaining=['Foreground plant domain requires independent semantic audit.',
                   'Archival terrain support is provisional; inferred reverse volume needs review.']), indent=2)+'\n')
    sys.argv = ['prepare_branch', '--', '--revision', '10']
    prepare_branch.main()


if __name__ == '__main__':
    main()
