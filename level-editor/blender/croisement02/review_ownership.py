"""Revise unfinished ownership without changing any frozen worker catalog."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json


def main():
    current = OUT / 'ownership-revision/catalog.json'
    if current.exists() and json.loads(current.read_text()).get('version') == 2:
        raise ValueError('Initial ownership migration is superseded by the canonical authored-scenery catalog; revise the current catalog instead of rebuilding the historical one')
    source = OUT / 'catalog.json'
    catalog = json.loads(source.read_text())
    original = {g['id']: json.loads(json.dumps(g)) for g in catalog['groups']}
    groups = {g['id']: g for g in catalog['groups']}
    approved = {r['asset_id'] for r in json.loads((OUT / 'user-feedback.json').read_text())['records']
                if r['decision'] == 'approved'}
    revisions = {
        'tree-03': [50, 51],
        'northwest-rock-outcrop': [35, 36, 133],
        'southwest-rock-outcrop': [43, 131, 136, 137],
    }
    for slug, parts in revisions.items():
        asset = 'croisement02-' + slug
        if asset in approved:
            raise ValueError('Cannot change approved ownership: ' + asset)
        group = groups[asset]
        group['parts'] = [dict(obstacle=i, name=f"{group['name']} part {i:03}") for i in parts]
    root = groups['croisement02-west-root-bank']
    if root['id'] in approved:
        raise ValueError('Cannot reclassify approved root bank')
    root.update(id='croisement02-tree-21', name='Northern Boundary Tree 21', wood_mask=21,
                previous_id='croisement02-west-root-bank',
                classification='Source wood mask 21 and obstacle 132; previously mislabeled as a terrain bank')
    catalog['groups'].append(dict(id='croisement02-west-covered-state', name='West Covered State',
        parts=[dict(obstacle=144, name='West covered obstacle 144')]))
    for group in catalog['groups']:
        if 'state' in group['id']:
            group['state_only'] = True
            group['classification'] = 'Patch-controlled obstacle metadata; visible state sprites reviewed separately'
    parts = [p['obstacle'] for g in catalog['groups'] for p in g['parts']]
    assert sorted(parts) == list(range(150))
    for asset in approved:
        assert next(g for g in catalog['groups'] if g['id'] == asset) == original[asset]
    catalog['review_notes'] += ' Unfinished ownership revised against source coordinates, native wood masks, and patch old/new obstacle lists.'
    directory = OUT / 'ownership-revision'
    directory.mkdir(exist_ok=True)
    write_json(directory / 'catalog.json', catalog)
    inventory = OUT / 'forest-v4-inventory/inventory.json'
    write_json(directory / 'grouping-review.json', dict(status='reviewed', reviewer='Codex',
        catalog_sha256=sha(directory / 'catalog.json'), inventory_sha256=sha(inventory),
        previous_catalog_sha256=sha(source),
        evidence=['Obstacle 133 projects onto the northwest edge rock, hundreds of pixels from the southwest group.',
                  'Obstacle 132 follows the northern trunk in native wood mask 21, not a west terrain bank.',
                  'Obstacle 144 and mask 137 are independently switched by native patch 006.',
                  'All 150 native parts retain one owner; approved groups are unchanged.']))
    print(directory / 'catalog.json')


if __name__ == '__main__':
    main()
