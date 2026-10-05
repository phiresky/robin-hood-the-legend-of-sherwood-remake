"""Archive an explicit batch approval against a hash-pinned texture gallery."""
import argparse
import json
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
from texture_decisions import evidence, fields, sha


def record(snapshot, gallery, decisions, expected_hash, count, user_text, receipt):
    if sha(snapshot) != expected_hash:
        raise ValueError('Frozen gallery hash differs')
    document = json.loads(snapshot.read_text())
    items = document['items']
    if len(items) != count or len({item['id'] for item in items}) != count:
        raise ValueError('Frozen approval scope differs')
    previous_bytes = decisions.read_bytes()
    previous = json.loads(previous_bytes)
    records = previous['decisions']
    prepared = []
    for item in items:
        if item['status'] != 'ready-for-user' or item['user_approval'] != 'pending':
            raise ValueError('Frozen batch includes a nonpending candidate')
        paths, hashes = evidence(item)
        images, reports = fields(item)
        for key in (*images, *reports):
            displayed = item['images' if key in images else 'reports'][key]
            if hashes[key] != displayed['sha256'] or sha(gallery / displayed['file']) != hashes[key]:
                raise ValueError('Displayed frozen evidence changed: ' + item['id'] + '/' + key)
        archive = decisions.parent / 'approved-evidence' / item['id'] / item['review_revision']
        decision = dict(asset_id=item['id'], scope='texture', decision='approved',
                        review_revision=item['review_revision'], exact_user_text=user_text,
                        evidence_sha256=hashes, evidence_paths={k: str(v) for k, v in paths.items()},
                        archive=str(archive), scope_resolution=f'Exactly {count} pending texture revisions in the frozen request; historical approvals and other galleries excluded',
                        approval_snapshot=str(snapshot), approval_snapshot_sha256=expected_hash)
        if item.get('texture_states'):
            decision['texture_states'] = item['texture_states']
        if archive.exists() or any(r['asset_id'] == item['id'] and r.get('review_revision') == item['review_revision'] for r in records):
            raise ValueError('Revision already archived or recorded; inspect instead of overwriting')
        prepared.append((paths, archive, decision))
    receipt.parent.mkdir(parents=True, exist_ok=True)
    backup = receipt.with_name('texture-decisions-before-approval.json')
    if backup.exists() or receipt.exists():
        raise FileExistsError('Approval receipt already exists')
    backup.write_bytes(previous_bytes)
    for paths, archive, decision in prepared:
        archive.mkdir(parents=True)
        for key, source in paths.items():
            target = archive / (key + source.suffix)
            shutil.copy2(source, target)
            if sha(target) != decision['evidence_sha256'][key]:
                raise ValueError('Archived evidence differs')
        (archive / 'decision.json').write_text(json.dumps(decision, indent=2) + '\n')
    if decisions.read_bytes() != previous_bytes or sha(snapshot) != expected_hash:
        raise ValueError('Decision stream or frozen snapshot changed concurrently')
    for paths, _, decision in prepared:
        if any(sha(path) != decision['evidence_sha256'][key] for key, path in paths.items()):
            raise ValueError('Approval evidence changed during archiving')
    result = dict(previous, decisions=records + [r for _, _, r in prepared])
    temporary = decisions.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(result, indent=2) + '\n')
    temporary.replace(decisions)
    receipt.write_text(json.dumps(dict(status='PASS', exact_user_text=user_text,
        snapshot=str(snapshot), snapshot_sha256=expected_hash, recorded=count,
        previous_records=len(records), historical_records_unchanged=True,
        decisions_sha256=sha(decisions), previous_decisions_sha256=sha(backup),
        revisions={r['asset_id']: r['review_revision'] for _, _, r in prepared}), indent=2) + '\n')
    print(receipt)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('snapshot', 'gallery', 'decisions', 'receipt'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--expected-hash', required=True)
    parser.add_argument('--count', required=True, type=int)
    parser.add_argument('--user-text', required=True)
    args = vars(parser.parse_args())
    record(**{k: v.resolve() if isinstance(v, Path) else v for k, v in args.items()})
