"""Stage the exact approved mechanism contract for normal catalog verification."""
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2'
SOURCE = BASE / 'restart63-winch-delivery-v2'
OUT = BASE / 'restart71-winch-catalog-stage-v1'
LIB = ROOT / 'level-editor/library'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    handoff = SOURCE / 'handoff.json'
    assert sha(handoff) == '7295f58c052c120451ae4125860cb83282bed8f1b6fe7c35160a14b394894894'
    receipt = ROOT / 'level-editor/work/croisement02-refinement/restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json'
    assert sha(receipt) == 'd435a76b08eb3ac616f1ae2ffcdf19a7875969e515dd01f65de1871ca886ef1d'
    data = json.loads(handoff.read_text())
    source = SOURCE / 'library'
    entry = data['entry']
    resources = {r['path']: r['sha256'] for r in data['resources']}
    resources[entry['contract']['path']] = entry['contract']['sha256']
    index_path = LIB / 'mission-states/index.json'
    index_sha = sha(index_path)
    index = json.loads(index_path.read_text())
    assert all(r['id'] != entry['id'] for r in index['entries'])
    assert not OUT.exists(), 'Fresh output required'
    stage = OUT / 'library'
    records = []
    for rel, digest in sorted(resources.items()):
        assert Path(rel).is_relative_to('mission-states/york')
        path = source / rel
        assert sha(path) == digest, rel
        target = stage / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        os.link(path, target)
        records.append({'relative': rel, 'source': str(target), 'sha256': digest,
                        'bytes': target.stat().st_size})
    index['entries'].append(entry)
    dest = stage / 'mission-states/index.json'
    dest.write_text(json.dumps(index, separators=(',', ':')) + '\n')
    assert sha(index_path) == index_sha, 'Concurrent catalog drift'
    records.append({'relative': 'mission-states/index.json', 'source': str(dest),
                    'sha256': sha(dest), 'bytes': dest.stat().st_size})
    plan = {'status': 'APPROVED_BYTES_PENDING_NORMAL_EDITOR_PROOF', 'library': str(stage),
            'entry': entry, 'records': records, 'approval': {'path': str(receipt), 'sha256': sha(receipt)},
            'baseline_catalog_sha256': index_sha,
            'scope': 'Independent patch004 mechanism only; room005 and gate000 remain independent. '
                     'Exact approved appearance, geometry and all45 poses. No canonical writes. '
                     'Final transaction must rebase this single entry under publication lock.'}
    (OUT / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(json.dumps({'output': str(OUT), 'records': len(records), 'plan_sha256': sha(OUT / 'plan.json')}))


if __name__ == '__main__':
    main()
