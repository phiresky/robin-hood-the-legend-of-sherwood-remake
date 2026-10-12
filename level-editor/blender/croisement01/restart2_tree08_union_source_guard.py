"""Independently compare authoritative rays after a branch union trial."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from vtkmodules.vtkCommonCore import reference
from vtkmodules.vtkCommonDataModel import vtkGenericCell, vtkStaticCellLocator
from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit

R = Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2'
out = R/(sys.argv[1] if len(sys.argv)>1 else 'tree08-inferred-shell-union-v1')
assert out.resolve().parent == R.resolve()
before = np.load(R/'tree08-source-guarded-fairing-cpu-v2-oriented/candidate.npz')
after = np.load(out/'candidate.npz')
s, c = np.sin(np.radians(35)), np.cos(np.radians(35))
ray, down = np.array([0., -c, s]), np.array([0., -s, -c])
pixels = np.argwhere(np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png')) > 0)
positions = np.array([np.array([x+331.5, 0, 0])+down*(y+11.5) for y,x in pixels])
def trace(data):
    locator = vtkStaticCellLocator()
    locator.SetDataSet(poly(data['vertices'], data['faces']))
    locator.BuildLocator()
    cell = vtkGenericCell()
    hits = []
    for point in positions:
        t, sub, face = reference(0.), reference(0), reference(0)
        hit, uvw = [0., 0., 0.], [0., 0., 0.]
        found = locator.IntersectWithLine(point+ray*2000, point-ray*2000, 1e-9, t, hit, uvw, sub, face, cell)
        hits.append(hit if found else [float('nan')]*3)
    return np.array(hits)
error = np.linalg.norm(trace(after)-trace(before), axis=1)
topology = audit(after['vertices'], after['faces'])
report = dict(source_pixels=len(pixels), lost_pixels=int(np.isnan(error).sum()), changed_source_hits=int((error>.0002).sum()), maximum_source_hit_error=float(np.nanmax(error)), topology=topology)
report['status'] = 'PASS_SOURCE_RAYS_AND_TOPOLOGY' if not(report['lost_pixels'] or report['changed_source_hits'] or any(topology[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])) else 'HOLD_SOURCE_OR_TOPOLOGY'
report['limits'] = 'Strict intersections, contact, source RGB/UV reconstruction, anatomy and saved views still required.'
(out/'source-guard.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2), flush=True)
