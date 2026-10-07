"""Choose an explicitly inferred equal-and-opposite chain phase trajectory."""
import hashlib
import json
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2'
SOURCE = BASE / 'winch-strand-motion-v2.json'
OUT = BASE / 'winch-supported-motion-plan-v1.json'
if OUT.exists():
    raise FileExistsError(OUT)
source = json.loads(SOURCE.read_text())
values = np.arange(-112, 113, dtype=float) / 8
modulo = lambda v: round(float((v + 3.5) % 7 - 3.5), 3)
cost = values**2 * .2
pointers, scores_by_frame = [], []
for row in source['rows']:
    lookup = {side: {round(r['down_pixels_modulo7'], 3): r['correlation']
                     for r in row['sides'][side]['all_scores']} for side in ('left', 'right')}
    scores = np.array([(lookup['left'][modulo(-v)] + lookup['right'][modulo(v)]) / 2 for v in values])
    matrix = cost[:, None] + .15 * (values[:, None] - values[None, :])**2
    pointer = matrix.argmin(axis=0)
    cost = matrix[pointer, np.arange(len(values))] + 1 - scores + .002 * values**2
    pointers.append(pointer)
    scores_by_frame.append(lookup)
state = int(cost.argmin())
steps = []
for pointer in reversed(pointers):
    steps.append(float(values[state]))
    state = int(pointer[state])
steps.reverse()
# Preserve the exact frozen final material-link positions, not just the pattern.
phases = [3.5 - sum(steps)]
for step in steps:
    phases.append(phases[-1] + step)
assert len(phases) == 45 and phases[-1] == 3.5
rows = [{'frame': i, 'tick': i * 2, 'phase_native_pixels': phase,
         'phase_modulo7': phase % 7,
         'right_down_step': steps[i - 1] if i else None,
         'left_up_step': steps[i - 1] if i else None} for i, phase in enumerate(phases)]
for i, row in enumerate(rows[1:]):
    row['left_correlation'] = scores_by_frame[i]['left'][modulo(-steps[i])]
    row['right_correlation'] = scores_by_frame[i]['right'][modulo(steps[i])]
result = {
    'status': 'Private inferred animation proposal, no model changed or accepted motion',
    'source_phase_evidence_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    'source_model_sha256': hashlib.sha256((BASE / 'winch-supported-hardware-v1/model.blend').read_bytes()).hexdigest(),
    'method': 'Joint smooth modulo-seven phase representative with equal and opposite strand speeds; same speed/acceleration regularization as source diagnostic',
    'chain_count': 76, 'link_spacing_native_pixels': 3.5, 'pattern_period_native_pixels': 7,
    'link_path': 'Unchanged supported-hardware76 path; positive path phase moves left up and right down',
    'total_right_down_pixels': sum(steps), 'rows': rows,
    'mean_correlations': {side: float(np.mean([r[side + '_correlation'] for r in rows[1:]])) for side in ('left', 'right')},
    'preservation': ['All24 body components and45 measured/inferred traveller/crank poses unchanged.',
                     'Nine supported hardware meshes unchanged; collar and arm follow retained traveller parent.',
                     'Final material-link transforms retained by exact phase3.5.',
                     'Constant keys at two-tick source spacing; no unvalidated transform interpolation.'],
    'limitations': ['Modulo-seven aliasing cannot establish true chain travel or gearing.',
                    'Sliding collar and independent traveller restraint remain inferred; no force simulation claimed.',
                    'Smoothness is a numerical selection criterion, not observed acceleration.',
                    'Finite phase/pose intersection checks do not prove continuous clearance.',
                    'Existing source-hole discrepancies remain; this fit does not optimize isolated holes.']}
OUT.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'total': sum(steps), 'mean_correlations': result['mean_correlations'], 'steps': steps}))
