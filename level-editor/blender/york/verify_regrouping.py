"""Check a review-driven regroup against the preceding delivered geometry."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/active/york'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--audit', type=Path, default=Path(__file__).with_name('grouping-audit-round-2.json'))
    parser.add_argument('--output', type=Path, default=OUT/'review-round-2/preservation.json')
    args = parser.parse_args()
    current = OUT / 'review/geometry.json'
    before = json.loads(args.before.read_text())
    after = json.loads(current.read_text())
    audit = json.loads(args.audit.read_text())
    floor_changes = set(audit.get('floor_changes', []))
    affected = set(floor_changes)
    for change in audit['changes']:
        affected.add(change['from'])
        if 'to' in change:
            affected.add(change['to'])
        affected.update(change.get('components', {}).values())
    unchanged = sorted(set(before) - affected)
    for identity in unchanged:
        if before[identity] != after.get(identity):
            raise ValueError('Unexpected geometry/name change outside review corrections: '+identity)
    removed = sorted(set(before) - set(after))
    added = sorted(set(after) - set(before))
    assert set(removed+added) <= affected
    # Whole-source moves must preserve the preceding grounded triangles and
    # positions, allowing only float32 pivot roundoff. Partitioned sources have
    # separate area, retained-surface and UV proofs.
    def parts(doc):
        return {(p['source_node'], p['component']): p for g in doc.values() for p in g['parts']}
    old, new = parts(before), parts(after)
    regrounded = {(p['source_node'],p['component']) for identity,record in after.items()
                  if identity in floor_changes for p in record['parts']}
    errors = []
    checked = 0
    repartitioned = []
    max_drift = 0
    for key in old.keys() & new.keys():
        if key in regrounded:
            continue  # Retained surfaces and UVs are checked by verify_grounding.
        a, b = old[key], new[key]
        if key[0] in audit.get('repartitioned_sources', []) and a != b:
            repartitioned.append(key)
            continue  # Area and UV coverage are checked by verify_partition.
        if a['triangles'] != b['triangles'] or len(a['positions']) != len(b['positions']):
            errors.append({'part': key, 'reason': 'Topology changed'})
            continue
        drift = max((abs(x-y) for p,q in zip(a['positions'], b['positions']) for x,y in zip(p,q)), default=0)
        if drift > .003:
            errors.append({'part': key, 'reason': 'Positions changed', 'drift': drift})
        max_drift = max(max_drift, drift)
        checked += 1
    result = {'status': 'FAIL' if errors else 'PASS', 'before_sha256': sha(args.before),
              'after_sha256': sha(current), 'unchanged_assets': unchanged,
              'changed_existing_assets': sorted(set(after) & affected & set(before)),
              'added_assets': added, 'retired_assets': removed,
              'preserved_components': checked, 'regrounded_components':len(regrounded),
              'repartitioned_components':sorted(repartitioned),
              'max_position_drift': max_drift, 'errors': errors}
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:len(v) if isinstance(v,list) and k!='errors' else v for k,v in result.items()}))
    if errors:
        raise ValueError('Regrouping changed pre-existing grounded surfaces')


if __name__ == '__main__':
    main()
