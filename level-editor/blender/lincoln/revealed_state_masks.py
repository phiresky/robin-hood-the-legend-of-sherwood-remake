"""Build reviewed per-state receiver masks for one Lincoln asset (system Python).

python3 level-editor/blender/lincoln/revealed_state_masks.py <spec.json> <workspace> <out-dir>

For each state in the spec, a new projection label ``state-<id>`` is written with
one explicit assignment per owned source node (and per covered-state component
assignment). The rule is:

    state mask = (covered mask AND NOT change) OR (owned state region AND change)

``change`` is the exact set of pixels whose RGB differs between the state source
and covered.png (revealed_state_sources.py). Outside it, the covered review is
unchanged by construction. Inside it, the spec's ``state_regions`` polygons
(pixel coordinates, reviewed against the state artwork) say which part of the
changed art belongs to this asset; ``"all"`` claims the whole change region
(single-owner patches only). ``state_region_excludes`` removes explicitly
reviewed foreground polygons (for example a neighbour's retained wall) from it.
Unconstrained covered nodes stay unconstrained outside the change region.
First-hit visibility still decides which owned mesh receives each pixel.
"""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/lincoln-refinement'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def polygon_mask(size, polygons):
    image = Image.new('L', size)
    draw = ImageDraw.Draw(image)
    for polygon in polygons:
        draw.polygon([tuple(p) for p in polygon], fill=255)
    return np.asarray(image) > 0


def main():
    spec_path, workspace, out_dir = map(Path, sys.argv[1:4])
    spec = json.loads(spec_path.read_text())
    config = json.loads((workspace / 'workspace.json').read_text())
    if config['asset_id'] != spec['asset_id']:
        raise ValueError('Spec and workspace assets differ')
    sources = json.loads((WORK / 'state-review/sources/manifest.json').read_text())
    covered_manifest_path = Path(config['source_mask_manifest']).resolve()
    covered = json.loads(covered_manifest_path.read_text())
    inventory_path = (covered_manifest_path.parent / covered['mask_inventory']).resolve()
    inventory = json.loads(inventory_path.read_text())
    records = {r['index']: r for r in inventory['masks']}
    width, height = Image.open(WORK / 'source-states/covered.png').size
    cache = {}

    def native(index):
        if index not in cache:
            record = records[index]
            left, top = record['box_top_left']
            w, h = record['box_size']
            path = inventory_path.parent / (record['png'] if 'png' in record else record['folder'] + '/mask.png')
            rgba = np.asarray(Image.open(path).convert('RGBA'))
            bitmap = (rgba[:, :, :3].max(axis=2) > 127) & (rgba[:, :, 3] > 127)
            if bitmap.shape != (h, w):
                raise ValueError(f'Mask {index} size differs from inventory box')
            full = np.zeros((height, width), bool)
            x0, y0 = max(left, 0), max(top, 0)
            x1, y1 = min(left + w, width), min(top + h, height)
            full[y0:y1, x0:x1] = bitmap[y0 - top:y1 - top, x0 - left:x1 - left]
            cache[index] = full
        return cache[index]

    exterior = covered['projections']['exterior']
    owned_nodes = set(config['part_ids'])
    targets = []
    for assignment in exterior['assignments']:
        if assignment.get('source_node') in owned_nodes or assignment.get('asset_group') == spec['asset_id']:
            targets.append(assignment)
    assigned = {a.get('source_node') for a in targets if 'projection_component' not in a}
    out = out_dir.resolve()
    masks_dir = out / 'masks'
    masks_dir.mkdir(parents=True, exist_ok=True)
    new_records, projections, review = [], {}, {}
    next_index = 0
    for state, patches in spec['states'].items():
        source = sources['states'][state]
        if source['applied_patches'] != patches:
            raise ValueError('State source patch set differs for ' + state)
        change = np.asarray(Image.open(source['change_region'])) > 0
        region_spec = spec.get('state_regions', {}).get(state, 'all')
        own = change.copy() if region_spec == 'all' else polygon_mask((width, height), region_spec) & change
        excludes = spec.get('state_region_excludes', {}).get(state, [])
        if excludes:
            own &= ~polygon_mask((width, height), excludes)
        rows = []
        entries = list(targets) + [{'source_node': node, 'unconstrained_covered': True}
                                   for node in sorted(owned_nodes - assigned)]
        for assignment in entries:
            if assignment.get('unconstrained_covered'):
                base = np.ones((height, width), bool)
            else:
                base = np.zeros((height, width), bool)
                for index in assignment['mask_indices']:
                    base |= native(index)
                for index in assignment.get('exclude_mask_indices', []):
                    base &= ~native(index)
            bitmap = (base & ~change) | (own & change)
            ys, xs = np.nonzero(bitmap)
            if not len(xs):
                raise ValueError('Empty state mask for ' + str(assignment.get('source_node')))
            left, top, right, bottom = int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1
            name = f"{state}-{next_index:04d}.png"
            Image.fromarray((bitmap[top:bottom, left:right] * 255).astype(np.uint8)).save(masks_dir / name)
            new_records.append({'index': next_index, 'png': 'masks/' + name,
                                'box_top_left': [left, top], 'box_size': [right - left, bottom - top]})
            row = {'reviewed': True, 'mask_indices': [next_index],
                   'constraint_kind': 'reviewed-state-receiver',
                   'review_note': ('Covered mask outside the state change region; inside it, the reviewed '
                                   f'{spec["asset_id"]} ownership of the {state} artwork.'),
                   'covered_assignment': {k: v for k, v in assignment.items()
                                          if k in ('source_node', 'asset_group', 'projection_component',
                                                   'mask_indices', 'exclude_mask_indices', 'constraint_kind',
                                                   'unconstrained_covered')}}
            for key in ('source_node', 'asset_group', 'projection_component'):
                if key in assignment:
                    row[key] = assignment[key]
            rows.append(row)
            next_index += 1
        projections['state-' + state] = {
            'source_sha256': source['sha256'], 'source_image': source['image'],
            'state': f'{state}: applied {", ".join(patches)}; every other base patch at its initial graphic',
            'assignments': rows}
        preview = Image.open(source['image']).convert('RGB')
        x0, y0, x1, y1 = source['change_bbox']
        pad = 24
        box = (max(0, x0 - pad), max(0, y0 - pad), min(width, x1 + pad), min(height, y1 + pad))
        crop = np.asarray(preview.crop(box)).astype(float)
        c = change[box[1]:box[3], box[0]:box[2]]
        o = own[box[1]:box[3], box[0]:box[2]]
        crop[c & ~o] = crop[c & ~o] * .35 + np.array([255, 0, 0]) * .65
        edge = np.zeros_like(o)
        edge[1:] |= o[1:] != o[:-1]
        edge[:, 1:] |= o[:, 1:] != o[:, :-1]
        crop[edge] = (0, 255, 255)
        evidence = out / f'{state}-ownership.png'
        Image.fromarray(crop.astype(np.uint8)).save(evidence)
        review[state] = {'change_pixels': int(change.sum()), 'owned_change_pixels': int(own.sum()),
                         'foreign_change_pixels': int((change & ~own).sum()),
                         'region': region_spec if region_spec == 'all' else 'polygons',
                         'evidence': str(evidence)}
    (out / 'inventory.json').write_text(json.dumps({'version': 1, 'masks': new_records}, indent=2) + '\n')
    manifest = {'version': 1, 'map': 'Lincoln', 'mask_inventory': 'inventory.json',
                'derived_from': str(covered_manifest_path), 'derived_from_sha256': sha(covered_manifest_path),
                'spec': str(spec_path.resolve()), 'spec_sha256': sha(spec_path),
                'rule': __doc__.split('\n\n')[1].strip(),
                'projections': projections, 'review': review}
    (out / 'source-masks.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(review))


if __name__ == '__main__':
    main()
