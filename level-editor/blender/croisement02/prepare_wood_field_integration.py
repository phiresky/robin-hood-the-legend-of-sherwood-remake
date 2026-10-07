"""Prepare bounded CPU field/collar inputs; never open or save Blender models.

The packet is a construction input, not a geometry review. Exact old boundary
vertices remain in the new correspondence; the eventual local mesh operation
must split existing edges and interpolate their UV data, without moving them.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.spatial import cKDTree

from continuous_wood_field import thickness_field, shell, cut_shell_below, boundary_normals
from fit_wood_collars_cpu import fit

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement'
STUDY = OUT / 'wood-sweep-cpu-review'
CONFIG = {32: (115., 125., (1030, 610, 1141, 757)),
          38: (105., 115., (1550, 585, 1611, 714))}
RETAINED_SHA = '044b416aa1d206dd87151a0358f08d42445876b4901a135e34b1dca417929b73'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source(index, box):
    if index == 32:
        manifest = OUT / 'baseline/masks/manifest.json'
        record = next(r for r in json.loads(manifest.read_text())['masks'] if r['index'] == index)
        path = manifest.parent / record['png']
        bitmap = np.asarray(Image.open(path).convert('L')) > 0
        ox, oy = record['box_top_left']
    else:
        path = OUT / 'tree38-root-research/source-domain-v1/wood38-plus-reviewed-contour.png'
        bitmap = np.asarray(Image.open(path).convert('L')) > 0
        ox = oy = 0
    x0, y0, x1, y1 = box
    return bitmap[y0-oy:y1-oy, x0-ox:x1-ox], path


def edge_positions(original, new):
    """Locate every new collar point on an actual old edge, not a best-fit ring."""
    old = np.asarray(original)
    delta = np.roll(old, -1, axis=0) - old
    length2 = np.sum(delta * delta, axis=1)
    result = []
    for point in new:
        t = np.clip(np.sum((point-old)*delta, axis=1)/length2, 0., 1.)
        error = np.linalg.norm(old+t[:, None]*delta-point, axis=1)
        edge = int(np.argmin(error))
        if error[edge] > 1e-7:
            raise ValueError('Collar endpoint is not on the preserved boundary')
        result.append([edge, float(t[edge])])
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--smooth-rim', action='store_true')
    parser.add_argument('--tree', type=int, choices=[32,38])
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    retained_path = STUDY / 'retained-collars-v1.json'
    if sha(retained_path) != RETAINED_SHA:
        raise ValueError('Retained boundary evidence changed')
    retained = json.loads(retained_path.read_text())
    ground_manifest = OUT / 'forest-v4-sources/manifest.json'
    ground = {r['mask']: r['ground_y'] for r in json.loads(ground_manifest.read_text())}
    arrays = {}
    records = []
    for index, (lower_z, upper_z, box) in CONFIG.items():
        if args.tree is not None and args.tree != index:continue
        old = next(r for r in retained['records'] if r['tree'] == index)
        model = Path(old['worker']) / 'model.blend'
        if sha(model) != old['model_sha256']:
            raise ValueError('Pinned private input changed')
        upper = next(c for c in old['cuts'] if c['height'] == upper_z)
        observed, source_path = source(index, box)
        body, thickness, coverage = thickness_field(observed)
        rim_report = None
        if args.smooth_rim:
            from smooth_wood_field import smooth_shell
            vertices, faces, rim_report = smooth_shell(body, thickness, box[:2], ground[index])
        else:
            vertices, faces = shell(body, thickness, box[:2], ground[index])
        vertices, faces, loops = cut_shell_below(vertices, faces, lower_z)
        # Eliminate unused points left above the clipping plane.
        used = np.unique(faces)
        remap = np.full(len(vertices), -1, int)
        remap[used] = np.arange(len(used))
        vertices, faces, loops = vertices[used], remap[faces], [remap[l] for l in loops]
        if len(loops) != len(upper['ordered_loops']):
            raise ValueError('Retained and new branch counts disagree')
        prefix = f'tree{index}'
        arrays[prefix+'_vertices'] = vertices
        arrays[prefix+'_faces'] = faces
        available = set(range(len(upper['ordered_loops'])))
        collars = []
        for number, loop in enumerate(loops):
            lo = dict(positions=vertices[loop], normals=boundary_normals(vertices, faces, loop))
            upper_index = min(available, key=lambda i: np.linalg.norm(np.mean([upper['vertices'][str(v)]['position'] for v in upper['ordered_loops'][i]], axis=0)-vertices[loop].mean(axis=0)))
            available.remove(upper_index)
            hi_ids = upper['ordered_loops'][upper_index]
            hi = dict(positions=[upper['vertices'][str(v)]['position'] for v in hi_ids], normals=[upper['vertices'][str(v)]['geometric_normal'] for v in hi_ids])
            fitted = fit(lo, hi, include_geometry=True, tangent_mode="up-projection",tangent_smoothing=.5 if args.smooth_rim else 0.)
            if args.smooth_rim and not fitted['eligible_for_bounded_integration']:
                initial_phase=fitted['lower_phase'];phase_attempts=[]
                for offset in [v/1024 for i in [1,2,4,8,16,32] for v in [i,-i]]:
                    trial=fit(lo,hi,include_geometry=True,tangent_mode='up-projection',forced_phase=initial_phase+offset,tangent_smoothing=.5)
                    phase_attempts.append(dict(offset=offset,passed=trial['eligible_for_bounded_integration'],quality=trial['quality']))
                    if trial['eligible_for_bounded_integration']:
                        fitted=trial;fitted['phase_search']=phase_attempts;break
            if not fitted['eligible_for_bounded_integration']:
                raise ValueError(f'Collar guard failed for {index}/{number}: '+json.dumps({k:v for k,v in fitted.items() if k!='geometry'}))
            geometry = fitted.pop('geometry')
            rows = np.asarray(geometry['rows'])
            key = f'{prefix}_collar{number}'
            arrays[key+'_rows'] = rows
            arrays[key+'_parameters'] = np.asarray(geometry['parameters'])
            for side in ['lower', 'upper']:
                original = np.asarray(geometry[side+'_original'])
                arrays[key+'_'+side+'_original'] = original
                arrays[key+'_'+side+'_edge_positions'] = np.asarray(edge_positions(original, rows[0 if side == 'lower' else -1]))
            # Record actual field vertex IDs in the normalized correspondence order.
            errors, positions = cKDTree(vertices).query(geometry['lower_original'])
            if errors.max() > 1e-8:
                raise ValueError('Field boundary point not found')
            arrays[key+'_lower_original_ids'] = positions
            collars.append(dict(array_prefix=key, upper_extraction_loop=upper_index, **fitted))
        unresolved = np.argwhere(observed & ~body)[:, ::-1] + np.array(box[:2])
        records.append(dict(tree=index, input_model=str(model), input_model_sha256=old['model_sha256'], source_node=old['source_node'], source_box=box, source_mask=str(source_path), source_mask_sha256=sha(source_path), lower_cut_z=lower_z, upper_cut_z=upper_z, field_vertices=len(vertices), field_triangles=len(faces), coverage=coverage, rim_report=rim_report, unresolved_source_coordinates=unresolved.tolist(), collars=collars))
    args.output.mkdir(parents=True)
    payload = args.output/'field-collars.npz'
    np.savez_compressed(payload, **arrays)
    report = dict(status='CPU construction packet only; PRIVATE HOLD', payload=str(payload.resolve()), payload_sha256=sha(payload), retained_evidence=str(retained_path), retained_evidence_sha256=RETAINED_SHA, ground_manifest=str(ground_manifest), ground_manifest_sha256=sha(ground_manifest), records=records,
        integration_contract=[
            'Open only pinned private input; output a fresh private model. No canonical selection or approval carry.',
            'Open retained wood at upper cut without a cap. Preserve crown and all out-of-scope meshes/materials byte-equivalent.',
            'Subdivide actual retained and field boundary edges at recorded fractions. Keep original endpoints exact and interpolate UVs on retained faces.',
            'Join lower field to retained upper using corresponding collar rows, sharing vertices on both interfaces. No overlapping cap, voxel union or global remesh.',
            'Preserve separate native ownership32 on080/081/082 when assigning new local surfaces; do not change source masks.',
            'Preserve32 disconnected observed fragments separately or fail source coverage. Do not fabricate branch bridges.',
            'Require closed consistent topology, source rasterization and38 opaque-ground coverage before any geometry-readiness claim.',
            'Review solid and actual eight-view sheets for collar/fork artifacts; sampled collar Jacobian alone is insufficient.'])
    (args.output/'recipe.json').write_text(json.dumps(report, indent=2)+'\n')
    print(args.output, payload.stat().st_size)


if __name__ == '__main__':
    main()
