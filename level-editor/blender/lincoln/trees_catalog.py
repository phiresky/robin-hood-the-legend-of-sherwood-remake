"""Catalog, grouping review and source masks for the Lincoln tree/bush/scenery assets.

Plain Python, run from the repository root after trees_geometry.py:

    python3 level-editor/blender/lincoln/trees_catalog.py            # LINCOLN_TREES_SCENE=scene-v2 selects a rebuilt scene
    python3 level-editor/blender/lincoln/trees_catalog.py merge <base-catalog.json> <merged.json>

Writes into work/needs-review/lincoln-refinement/trees/ (never touches grouping/ or mask-review/):
  catalog-trees.json            catalog-v4 + one group per foliage/scenery asset (authored
                                scenery parts: node foliage-*/scenery-*, no obstacle)
  gallery-catalog.json          the same tree groups only, for a trees-only review gallery
  grouping-review-trees.json    review record binding catalog and scene inventory hashes
  masks/source-masks-trees.json + masks/inventory/manifest.json
                                masks v5 + one authored domain per asset (indices 456+),
                                assigned to its node; terrain receivers exclude the domains
                                (the proposed v6 ground cuts), and foliage occluder
                                constraints (foliage blocks others only inside its domain)
"""
import hashlib
import json
import os
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
OUT = R / 'trees'
PROPOSAL = R / 'scratch/trees/inventory/tree-catalog-proposal.json'
BASE_CATALOG = R / 'grouping/catalog-v4.json'
MASKS = R / 'mask-review/source-masks-v5.json'
SOURCE = R / 'source-states/covered.png'
SCENE_INVENTORY = OUT / os.environ.get('LINCOLN_TREES_SCENE', 'scene') / 'inventory/inventory.json'
FIRST_INDEX = 456
W, H = 2944, 2176


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def node_for(row):
    return ('scenery-' if row['kind'] == 'scenery' else 'foliage-') + row['id'].removeprefix('lincoln-')


def full(record, directory):
    bitmap = np.asarray(Image.open(directory / record['png']).convert('L')) > 0
    x, y = record['box_top_left']
    out = np.zeros((H, W), bool)
    out[y:y + bitmap.shape[0], x:x + bitmap.shape[1]] = bitmap[:H - y, :W - x]
    return out


def merged_catalog(base_path, output_path):
    """Append the tree groups to another reviewed catalog (e.g. a coordinator regroup).

    Refuses when the base already owns a foliage/scenery node or reuses a tree asset id.
    """
    trees = json.loads((OUT / 'catalog-trees.json').read_text())
    base = json.loads(Path(base_path).read_text())
    added = [g for g in trees['groups'] if g['parts'][0].get('node', '').startswith(('foliage-', 'scenery-'))]
    if {g['id'] for g in added} & {g['id'] for g in base['groups']}:
        raise ValueError('Base catalog already contains tree asset ids')
    if any(p['node'].startswith(('foliage-', 'scenery-')) for g in base['groups'] for p in g['parts'] if 'node' in p):
        raise ValueError('Base catalog already owns foliage/scenery nodes')
    merged = json.loads(json.dumps(base))
    merged['groups'] = base['groups'] + added
    merged['canonical_owners'] = {**base['canonical_owners'], **{g['parts'][0]['node']: g['id'] for g in added}}
    merged['revision_log'] = list(base.get('revision_log', [])) + [trees['revision_log'][-1]]
    Path(output_path).write_text(json.dumps(merged, indent=2) + '\n')
    return dict(base=str(base_path), base_sha256=sha(base_path), output=str(output_path),
                output_sha256=sha(output_path), groups=len(merged['groups']), added=len(added))


def main():
    import sys
    if len(sys.argv) == 4 and sys.argv[1] == 'merge':
        print(json.dumps(merged_catalog(Path(sys.argv[2]), Path(sys.argv[3]))))
        return
    proposal = json.loads(PROPOSAL.read_text())
    rows = proposal['assets']
    base = json.loads(BASE_CATALOG.read_text())
    groups = []
    for index, row in enumerate(rows):
        part = {'node': node_for(row), 'name': 'Painted ' + row['kind'] if row['kind'] != 'scenery'
                else 'Landing stage and mooring pole', 'foliage_domain_mask': FIRST_INDEX + index}
        groups.append({'id': row['id'], 'name': row['name'], 'kind': row['kind'], 'region': row['region'],
                       'parts': [part]})
    catalog = json.loads(json.dumps(base))
    catalog['groups'] = base['groups'] + groups
    catalog['canonical_owners'] = {**base['canonical_owners'], **{g['parts'][0]['node']: g['id'] for g in groups}}
    catalog['revision'] = base.get('revision', 0) + 1
    catalog['revision_log'] = list(base.get('revision_log', [])) + [
        f'Trees: adds {len(groups)} authored scenery assets (foliage-*/scenery-* parts, no sight obstacles) '
        'for painted trees, bushes and the pond landing stage; look approved by the user 2026-09-26.']
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'catalog-trees.json').write_text(json.dumps(catalog, indent=2) + '\n')
    gallery = {k: v for k, v in catalog.items() if k not in ('groups', 'canonical_owners')}
    gallery['groups'] = groups
    gallery['canonical_owners'] = {g['parts'][0]['node']: g['id'] for g in groups}
    (OUT / 'gallery-catalog.json').write_text(json.dumps(gallery, indent=2) + '\n')

    # Masks: v5 + authored domains; terrain receivers exclude the overlapping domains.
    masks_dir = OUT / 'masks'
    inventory_dir = masks_dir / 'inventory'
    inventory_dir.mkdir(parents=True, exist_ok=True)
    manifest = json.loads(MASKS.read_text())
    base_inventory_path = (MASKS.parent / manifest['mask_inventory']).resolve()
    base_inventory = json.loads(base_inventory_path.read_text())
    records = []
    for record in base_inventory['masks']:
        record = dict(record)
        if record.get('png'):
            record['png'] = os.path.relpath(base_inventory_path.parent / record['png'], inventory_dir)
        records.append(record)
    existing = {r['index'] for r in records}
    source_sha = sha(SOURCE)
    domains = {}
    for index, (row, group) in enumerate(zip(rows, groups)):
        mask_index = FIRST_INDEX + index
        if mask_index in existing:
            raise ValueError(f'Mask index collision: {mask_index}')
        name = f"authored-{mask_index}-{row['id'].removeprefix('lincoln-')}.png"
        Image.open(row['domain_png']).save(inventory_dir / name)
        size = Image.open(inventory_dir / name).size
        records.append({'index': mask_index, 'layer': None, 'layer_index': None, 'png': name, 'mask_type': None,
                        'box_top_left': row['domain_box_top_left'], 'box_size': list(size),
                        'character_polyline': [], 'projectile_polyline': [], 'obstacle_indices': [],
                        'synthetic': True, 'constraint_kind': 'reviewed-authored-receiver-domain',
                        'source_sha256': source_sha, 'review_evidence': str(PROPOSAL.relative_to(R)),
                        'reason': f"Painted {row['kind']} domain for {row['id']} ({row['domain_method']}; "
                                  f"native masks {row['native_masks']}).",
                        'pixels': row['domain_pixels']})
        domains[mask_index] = full(records[-1], inventory_dir)
        manifest['projections']['exterior']['assignments'].append({
            'reviewed': True, 'source_node': group['parts'][0]['node'], 'mask_indices': [mask_index],
            'constraint_kind': 'reviewed-authored-receiver-domain',
            'review_evidence': str(PROPOSAL.relative_to(R)),
            'review_note': f"{row['name']}: painted pixels; neighbouring crowns share pixels and first-hit decides."})
    # Terrain receivers: every row owning an authored ground domain or a terrain mask
    # from the trim proposal. Each excludes exactly the domains that overlap its
    # accepted mask (its masks minus its existing v5 exclusions), so no exclusion is a no-op.
    trims = json.loads((PROPOSAL.parent / 'ground-domain-trim.json').read_text())
    terrain_masks = {t['ground_mask'] for t in trims['trims']}
    by_index = {r['index']: r for r in records}
    changed = []
    for entry in manifest['projections']['exterior']['assignments']:
        if not set(entry['mask_indices']) & terrain_masks:
            continue
        accepted = np.zeros((H, W), bool)
        for index in entry['mask_indices']:
            accepted |= full(by_index[index], inventory_dir)
        for index in entry.get('exclude_mask_indices', []):
            accepted &= ~full(by_index[index], inventory_dir)
        overlaps = {i: int((domain & accepted).sum()) for i, domain in domains.items()}
        excluded = sorted(i for i, pixels in overlaps.items() if pixels)
        if not excluded:
            continue
        entry['exclude_mask_indices'] = sorted(set(entry.get('exclude_mask_indices', [])) | set(excluded))
        entry['exclusions_reviewed'] = True
        entry['exclusion_reason'] = ((entry.get('exclusion_reason', '') + ' ') if entry.get('exclusion_reason') else '') + \
            'Painted trees/bushes are separate foliage assets (authored domains 456+); their pixels leave the terrain receiver.'
        changed.append({'source_node': entry['source_node'], 'excluded_domains': excluded,
                        'overlap_pixels': {str(i): overlaps[i] for i in excluded}})
    # Inferred foliage geometry (wood, card depth) may only occlude other receivers
    # inside its painted domain; elsewhere the painting shows what is behind it.
    nodes = sorted({e['source_node'] for e in manifest['projections']['exterior']['assignments']})
    manifest['projections']['exterior']['occluder_constraints'] = [
        {'reviewed': True, 'source_node': group['parts'][0]['node'],
         'receiver_nodes': [n for n in nodes if n != group['parts'][0]['node']],
         'mask_indices': [FIRST_INDEX + index],
         'reason': 'Foliage/scenery geometry blocks foreign receivers only where its painted domain covers the pixel; '
                   'inferred wood and card depth outside it must not hide painted ground or architecture.',
         'review_evidence': str(PROPOSAL.relative_to(R))}
        for index, group in enumerate(groups)]
    (inventory_dir / 'manifest.json').write_text(json.dumps({**base_inventory, 'masks': records}, indent=2) + '\n')
    manifest['mask_inventory'] = 'inventory/manifest.json'
    manifest['limitations'] = list(manifest.get('limitations', [])) + [
        'Trees: adds authored foliage/scenery domains 456+ (one per catalog-trees asset) and excludes them from '
        'terrain receivers; proposed as masks v6 for the coordinator.']
    (masks_dir / 'source-masks-trees.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (masks_dir / 'terrain-exclusions.json').write_text(json.dumps(dict(version=1, changes=changed), indent=2) + '\n')

    review = {'status': 'reviewed',
              'reviewer': 'lincoln trees worker applying the user-approved tree look (2026-09-26) and the '
                          "team lead's instruction to add catalog entries for all tree/bush/scenery assets",
              'catalog_sha256': sha(OUT / 'catalog-trees.json'), 'inventory_sha256': sha(SCENE_INVENTORY),
              'base_catalog_sha256': sha(BASE_CATALOG),
              'notes': ['catalog-v4 groups unchanged; one new asset per painted plant grouping from '
                        'scratch/trees/inventory/tree-catalog-proposal.json, plus the pond landing stage.']}
    (OUT / 'grouping-review-trees.json').write_text(json.dumps(review, indent=2) + '\n')
    print(json.dumps(dict(groups=len(groups), masks=len(domains), terrain_assignments_changed=len(changed))))


if __name__ == '__main__':
    main()
