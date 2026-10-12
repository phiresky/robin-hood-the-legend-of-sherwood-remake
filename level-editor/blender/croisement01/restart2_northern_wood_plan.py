"""Measure the complete northern wood source group without asserting mask ownership."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement01'
OUT = BASE / 'restart2/northern-wood-construction-plan-v1'
IDS = (9, 10, 11, 12, 13, 15, 16, 17, 23)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    OUT.mkdir(exist_ok=False)
    source_path = BASE / 'baseline/covered.png'
    source = np.asarray(Image.open(source_path).convert('RGBA'))
    height, width = source.shape[:2]
    manifest_path = BASE / 'baseline/masks/manifest.json'
    masks = json.loads(manifest_path.read_text())['masks']
    domains = {}
    for row in masks:
        x, y = row['box_top_left']
        w, h = row['box_size']
        domain = np.zeros((height, width), dtype=bool)
        domain[y:y+h, x:x+w] = np.asarray(Image.open(BASE / 'baseline/masks' / row['png']).convert('L')) > 0
        domains[row['index']] = domain
    records = []
    for index in IDS:
        row = next(r for r in masks if r['index'] == index)
        domain = domains[index]
        ys, xs = np.nonzero(domain)
        folder = OUT / f'tree{index:02d}'
        folder.mkdir()
        box = [max(0, int(xs.min())-36), max(0, int(ys.min())-20), min(width, int(xs.max())+37), min(height, int(ys.max())+57)]
        Image.fromarray(source).crop(box).save(folder / 'context.png')
        isolated = source.copy()
        isolated[~domain] = 0
        Image.fromarray(isolated).crop(box).save(folder / 'native-source.png')
        sections = []
        for y in range(int(ys.min()), int(ys.max())+1):
            xx = np.flatnonzero(domain[y])
            if not len(xx):
                continue
            breaks = np.flatnonzero(np.diff(xx) > 1)
            starts = np.r_[0, breaks+1]
            ends = np.r_[breaks, len(xx)-1]
            sections.append({'y': y, 'runs': [[int(xx[a]), int(xx[b])+1] for a,b in zip(starts,ends)]})
        overlap = [{'mask': other['index'], 'mask_type': other['mask_type'], 'pixels': int((domain & domains[other['index']]).sum())}
                   for other in masks if other['index'] != index and np.any(domain & domains[other['index']])]
        record = {'native_mask': index, 'source_pixels': int(domain.sum()), 'crop': box,
                  'native_bounds': [int(xs.min()), int(ys.min()), int(xs.max())+1, int(ys.max())+1],
                  'touches_map_top': bool(domain[0].any()), 'row_runs': sections,
                  'mask_sha256': sha(BASE / 'baseline/masks' / row['png']),
                  'overlaps_not_exclusions': overlap,
                  'source_semantics': 'Native occupancy mask is a source candidate, not proof of exclusive physical ownership.',
                  'construction': ['Trace continuous wood and visible branch/root forks in unmarked context.',
                                   'Measure regional terrain at the source foot before assigning ground plane.',
                                   'Fit joined rounded cross sections to observed row runs; do not bridge separate fork runs blindly.',
                                   'Infer upper continuation and crown beyond y=0 using only the two permitted Leicester references.',
                                   'Keep observed artwork intact; independently separate crossing foreground where visually supported.',
                                   'Validate saved topology, all-view depth, actual first-hit ownership and bank contacts before review.'],
                  'unresolved': ['Regional terrain height', 'Physical crown association', 'Neighbor first-hit ordering', 'Inferred off-map shape'],
                  'geometry_status': 'NOT_BUILT_BY_THIS_PLAN'}
        (folder / 'plan.json').write_text(json.dumps(record, indent=2)+'\n')
        records.append({k:v for k,v in record.items() if k != 'row_runs'})
    report = {'status': 'MEASURED_SOURCE_GROUP_NOT_GEOMETRY_APPROVAL',
              'source_sha256': sha(source_path), 'mask_manifest_sha256': sha(manifest_path),
              'selected_first': 11, 'selection_reason': 'Native11 intersects the Crown94 context; refining its wood clarifies the adjacent complete Tree14 source ownership.',
              'records': records, 'references': ['Leicester southeast cottage tree', 'Leicester moat bank tree'],
              'limits': 'No physical depth or bark ownership inferred from mask overlap alone; no canonical changes.'}
    (OUT / 'plan.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'directory': str(OUT), 'trees': len(records), 'source_pixels': sum(r['source_pixels'] for r in records)}))


if __name__ == '__main__':
    main()
