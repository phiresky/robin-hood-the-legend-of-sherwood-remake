"""Merge isolated shrubs81/65/66 onto the fenced catalog without registering them."""
import argparse
import copy
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from catalog_schema import parse_catalog
from evidence_io import sha, write_json


def read(path):
    return json.loads(path.read_text())


def main(base, destination, indices):
    inventory_path=base/'inventory/inventory.json'
    if not inventory_path.exists():inventory_path=base/'inventory.json'
    previous_hash = sha(reviewed_catalog())
    previous = read(reviewed_catalog())
    if previous != read(base/'catalog.json'):
        raise ValueError('Current catalog differs from fenced integration base')
    grouping = read(OUT/'ownership-revision/grouping-review.json')
    if grouping['catalog_sha256'] != previous_hash or grouping['inventory_sha256'] != sha(inventory_path):
        raise ValueError('Canonical inventory binding differs')
    catalog = copy.deepcopy(previous)
    inventory = read(inventory_path)
    manifest = read(base/'source-masks.json')
    mask_path = Path(manifest['mask_inventory'])
    masks = read(mask_path)
    for row in masks['masks']:
        row['png'] = str((mask_path.parent/row['png']).resolve())
    inputs = {str(base/'catalog.json'):sha(base/'catalog.json'), str(inventory_path):sha(inventory_path), str(base/'source-masks.json'):sha(base/'source-masks.json')}
    nodes = []
    workers = {}
    domains = []
    for index, relative in [(81, 'southwest81-v1'), (65, 'north65-66-v1'), (66, 'north65-66-v1')]:
        if index not in indices:continue
        source = OUT/'understory-candidates'/relative
        asset = f'croisement02-shrub-{index:02}'
        node = f'foliage-shrub-{index:03}'
        group = next(g for g in read(source/'catalog.json')['groups'] if g['id'] == asset)
        if any(g['id'] == asset for g in catalog['groups']) or node in catalog['canonical_owners']:
            raise ValueError('Candidate is already integrated')
        if not group['authored_scenery'] or group['parts'][0]['node'] != node:
            raise ValueError('Candidate is not expected authored foliage')
        domain = group['parts'][0]['foliage_domain_mask']
        rows = [o for o in read(source/'inventory/inventory.json')['objects'] if o['source_node'] == node]
        if len(rows) != 1:
            raise ValueError('Expected one isolated foliage source object')
        catalog['groups'].append(group)
        catalog['canonical_owners'][node] = asset
        inventory['objects'] += rows
        incoming = read(source/'source-masks.json')
        incoming_path = Path(incoming['mask_inventory'])
        row = next(r for r in read(incoming_path)['masks'] if r['index'] == domain)
        if any(r['index'] == domain for r in masks['masks']):
            raise ValueError('Authored mask index collision')
        row['png'] = str((incoming_path.parent/row['png']).resolve())
        masks['masks'].append(row)
        assignments = incoming['projections']['exterior']['assignments']
        manifest['projections']['exterior']['assignments'].append(next(a for a in assignments if a.get('source_node') == node))
        constraints = incoming['projections']['exterior']['occluder_constraints']
        manifest['projections']['exterior']['occluder_constraints'].append(next(c for c in constraints if c['source_node'] == node))
        nodes.append(node); domains.append(domain)
        worker=OUT/'understory-round-1/assets'/asset
        refit=OUT/'understory-round-2/assets'/asset
        if (refit/'inspection/refit-evidence.json').exists():worker=refit
        workers[asset]=str(worker)
        for path in (source/'catalog.json', source/'inventory/inventory.json', source/'source-masks.json', Path(row['png'])):
            inputs[str(path)] = sha(path)
    receivers = {'ground', *catalog['canonical_owners']}
    for constraint in manifest['projections']['exterior']['occluder_constraints']:
        if constraint['source_node'].startswith(('foliage-', 'scenery-')):
            constraint['receiver_nodes'] = sorted(receivers-{constraint['source_node']})
    ground = next(a for a in manifest['projections']['exterior']['assignments'] if a.get('source_node') == 'ground')
    ground['exclude_mask_indices'] += domains
    ground['exclusion_reason'] += ' Pending shrubs81/65/66 retain their existing observed domains414/415/416.'
    parse_catalog(catalog, {o['source_node'] for o in inventory['objects']}-{'ground'})
    if sha(reviewed_catalog()) != previous_hash:
        raise ValueError('Canonical catalog changed during rebase')
    destination.mkdir(exist_ok=False)
    (destination/'inventory').mkdir()
    write_json(destination/'previous-catalog.json', previous)
    write_json(destination/'catalog.json', catalog)
    write_json(destination/'inventory/inventory.json', inventory)
    write_json(destination/'mask-inventory.json', masks)
    manifest['mask_inventory'] = str(destination/'mask-inventory.json')
    write_json(destination/'source-masks.json', manifest)
    write_json(destination/'grouping-review.json', dict(status='reviewed', reviewer='Codex',
        catalog_sha256=sha(destination/'catalog.json'), inventory_sha256=sha(destination/'inventory/inventory.json'),
        evidence='Merge only authored source ownership for shrubs81/65/66 onto the current fenced catalog. Existing group records, source nodes and observed domains are preserved. Geometry readiness and user decisions remain separate.'))
    write_json(destination/'workers.json',workers)
    write_json(destination/'rebase.json', dict(status='private proposal only; canonical catalog unchanged',
        previous_catalog_sha256=previous_hash, groups=len(catalog['groups']), added_nodes=nodes, domains=domains, inputs=inputs))
    print(destination)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base',type=Path,default=OUT/'fence-integration')
    parser.add_argument('--destination',type=Path,default=OUT/'understory-candidates/fence-rebase-v1')
    parser.add_argument('--indices',type=int,nargs='+',choices=[81,65,66],default=[81,65,66])
    args=parser.parse_args()
    main(args.base.resolve(),args.destination.resolve(),args.indices)
