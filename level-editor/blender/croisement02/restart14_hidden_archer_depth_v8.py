"""CPU-only support-envelope constraints; does not construct or render models."""
import argparse
import hashlib
import json
import math
import shutil
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial import cKDTree
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'work/croisement02-refinement/restart14-hidden-archer'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = np.array([0., -COS, SIN])

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def budget(dest):
    assert shutil.disk_usage(BASE).free >= 8 * 1024**3 + 2 * 1024**2
    assert sum(p.stat().st_size for p in dest.rglob('*') if p.is_file()) < 2 * 1024**2

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=BASE / 'depth-v8-cpu')
    args = parser.parse_args()
    dest = args.output
    budget(dest)
    dest.mkdir(exist_ok=False)
    measured = BASE / 'audit-v1/substrate-first-hit-v1/report.json'
    authority = BASE / 'audit-v1/source-authority.json'
    report = json.loads(measured.read_text())
    samples = next(p['samples'] for p in report['profiles'] if p['profile'].endswith('05'))
    source = next(p for p in json.loads(authority.read_text())['profiles'] if p['profile'].endswith('05'))
    lookup = {tuple(p['pixel']): p for p in samples}
    rock_samples = [p for p in samples if 'rock' in p['asset']]
    rock_xy = np.array([p['pixel'] for p in rock_samples])
    rock_z = np.array([p['world'][2] for p in rock_samples])
    rock_tree = cKDTree(rock_xy)
    records = []
    fig, axes = plt.subplots(2, 3, figsize=(15, 9), layout='constrained')
    for row, state in enumerate(source['states']):
        path = Path(state['sprite_source'])
        assert sha(path) == state['sprite_sha256']
        rgba = np.array(Image.open(path).convert('RGBA'))
        yy, xx = np.where(rgba[:, :, 3] >= 128)
        ox, oy = state['native_top_left']
        pixels = list(zip((xx + ox).tolist(), (yy + oy).tolist()))
        assert all(p in lookup and lookup[p]['world'] for p in pixels)
        support = np.array([lookup[p]['world'] for p in pixels])
        # Front leaves are 1.5 units along their exact native camera rays from
        # the actual support, not a max-filtered height field or deep extrusion.
        dist, near = rock_tree.query(np.array(pixels), k=8)
        weights = 1 / np.maximum(dist, .25)**2
        inferred_z = (rock_z[near]*weights).sum(1)/weights.sum(1)
        # Bank first hits are lower bounds, not foliage-height assignments.
        # Lift those rays toward the nearby rock envelope instead of laying
        # all bank-facing foliage onto the distant horizontal bank.
        target_z = np.maximum(support[:, 2], inferred_z) + 1.5*SIN
        front = support + RAY*((target_z-support[:, 2])/SIN)[:, None]
        projected = np.column_stack([front[:, 0], -front[:, 1] * SIN - front[:, 2] * COS])
        error = np.abs(projected - (np.array(pixels) + .5)).max()
        assert error < .005
        rock = np.array(['rock' in lookup[p]['asset'] for p in pixels])
        indices = {p: i for i, p in enumerate(pixels)}
        short, discontinuities = [], []
        for p, i in indices.items():
            for delta in [(1, 0), (0, 1)]:
                j = indices.get((p[0]+delta[0], p[1]+delta[1]))
                if j is None:
                    continue
                distance = float(np.linalg.norm(front[j]-front[i]))
                (short if distance <= 6 else discontinuities).append([i, j, distance])
        ax = axes[row, 0]
        ax.imshow(rgba, extent=[ox, ox+rgba.shape[1], oy+rgba.shape[0], oy])
        ax.set_title(state['state'] + ': exact native crop')
        ax = axes[row, 1]
        ax.scatter(np.array(pixels)[:, 0], np.array(pixels)[:, 1], c=np.where(rock, '#af7549', '#4d8b61'), s=2)
        ax.invert_yaxis(); ax.set_aspect('equal'); ax.set_title('Measured rock (brown) / bank (green)')
        ax = axes[row, 2]
        ax.scatter(support[:, 1], support[:, 2], c=np.where(rock, '#af7549', '#4d8b61'), s=2, label='Measured support')
        ax.scatter(front[:, 1], front[:, 2], c='#315ab5', s=.5, label='Inferred rock-following front')
        ax.set_aspect('equal'); ax.set_xlabel('World Y'); ax.set_ylabel('World Z'); ax.legend(fontsize=7)
        ax.set_title('Equal-scale Y/Z; blue is unvalidated inference', fontsize=9)
        record = dict(state=state['state'], source=str(path), source_sha256=sha(path), opaque_centers=len(pixels),
                      projection_error_max=float(error), support_world_min=support.min(0).tolist(), support_world_max=support.max(0).tolist(),
                      front_world_min=front.min(0).tolist(), front_world_max=front.max(0).tolist(), rock_centers=int(rock.sum()),
                      bank_centers=int((~rock).sum()), local_edges=len(short), rejected_long_edges=len(discontinuities),
                      maximum_rejected_distance=max([e[2] for e in discontinuities], default=0), baseline_depth_extent=float(np.ptp(support[:,1])), proposed_depth_extent=float(np.ptp(front[:,1])), inferred_lift_max=float((front[:,2]-support[:,2]).max()))
        budget(dest)
        (dest/f'{state["state"]}-constraints.json').write_text(json.dumps(dict(**record, pixels=pixels, support=np.round(support, 5).tolist(), front=np.round(front, 5).tolist(), rejected_edges=discontinuities), separators=(',', ':'))+'\n')
        records.append(record)
    budget(dest)
    fig.savefig(dest/'source-and-cross-sections.png', dpi=110)
    plt.close(fig)
    plan = dict(status='CPU CONSTRAINT PROPOSAL; NO MODEL OR GEOMETRY PASS', measured_report_sha256=sha(measured),
                frozen_scene_sha256=report['source_sha256'], authority_sha256=sha(authority), states=records,
                construction_rules=[
                    'Treat bank first-hit elevations as lower bounds, not foliage anchors; infer nearby rock-following height by local8 rock samples, then verify against actual selected rock triangles before accepting it.',
                    'Discard old generated interiors and max-filter shoulder; retain exact native RGBA/UV on camera-ray relocated front fragments.',
                    'The inferred nearest-rock envelope is a CPU proposal, not proof of rock clearance; preserve exact rays and reject impossible compact placement instead of hiding intersections.',
                    'Do not bridge any source-neighbor pair whose measured world separation exceeds 6 units. These are depth-discontinuity boundaries, not branch edges.',
                    'Append only the pinned rock035 and bank000 meshes for the next geometry audit. Extract a surface-geodesic path from rock foot to ledge; existing point samples do not prove the intervening face.',
                    'At most three rooted parent stems following that measured surface path, with local branches of at most 6 world units. No per-pixel stem forest or direct cliff-lip-to-bank chords.',
                    'Cluster front fragments within each contiguous support patch; inferred side leaves stay within 4 units of that patch and outside measured rock/bank interiors.',
                    'Hidden supports use explicit endpoint-alpha ownership, not source-RGB classification. Preserve every native opaque texel; source-transparent gaps stay transparent from the original camera.',
                    'Run all7073 center-RGBA tests plus native silhouette, rock penetration, stem connectivity and support tests before any all-eight views.',
                    'Reject rather than fill a geodesic discontinuity with a long strand. Sparse measured first-hit samples cannot establish closed rock clearance.'
                ], remaining_before_model='Extract selected receiver surface triangles and fit the few-stem geodesic envelope; validate opaque native centers against actual faces. This CPU proposal alone cannot certify contact.',
                limits=dict(total_output_bytes=128*1024**2, model_bytes=16*1024**2, minimum_free_bytes=10*1024**3, threads=2),
                image_sha256=sha(dest/'source-and-cross-sections.png'))
    budget(dest)
    (dest/'recipe.json').write_text(json.dumps(plan, indent=2)+'\n')
    budget(dest)
    assert len(list(dest.iterdir())) == 4
    print(json.dumps(records, indent=2))

if __name__ == '__main__':
    main()
