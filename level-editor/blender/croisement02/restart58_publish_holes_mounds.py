"""Rebase approved additive mission entries; root applies with readers quiescent."""
import argparse
import importlib.util
import json
import shutil
import subprocess
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'level-editor/library'
BASE = ROOT / 'level-editor/work/croisement02-refinement'
STAGE = BASE / 'restart53-hole-mound-transaction-v1'
RUNS = BASE / 'restart58-hole-mound-publication-v1'
HELPER = ROOT / 'level-editor/blender/york/restart44_static_transaction.py'
spec = importlib.util.spec_from_file_location('publication_io', HELPER)
io = importlib.util.module_from_spec(spec)
spec.loader.exec_module(io)
sha, require, dump = io.sha, io.require, io.dump
STAGE_SHA = '2d05f13370187c175dfa435065c2458f2b1d9199f08c9776864fee0cfae058b5'
PROOF = BASE / 'restart63-hole-mound-current-runtime-browser-v1/root-review-ready.json'
PROOF_SHA = '842ef6eb62d5755a86992d591c31e1172a2380e41123cd9c35ec75a9cf5c7127'
SELF_SHA = sha(Path(__file__))

def read(path):
    return json.loads(path.read_text())

def merge_catalog(current, additions):
    ids = [r['id'] for r in current['entries']]
    require(len(ids) == len(set(ids)), 'Duplicate existing catalog ids')
    require(len(additions) == 62 and len({r['id'] for r in additions}) == 62, 'Wrong additions')
    require(not set(ids).intersection(r['id'] for r in additions), 'Already installed or conflicting entries')
    return {**current, 'entries': [*current['entries'], *additions]}

def guards():
    require(sha(HELPER) == 'a71796d93dbfbc8fd07ebd6deebf8bc0177e7e599a85b9233c43147fd38a30e4', 'Helper drift')
    require(sha(STAGE/'transaction.json') == STAGE_SHA, 'Stage drift')
    require(sha(PROOF) == PROOF_SHA, 'Browser handoff drift')
    proof = read(PROOF)
    require(proof['status'] == 'PASS' and proof['transaction_sha256'] == STAGE_SHA, 'Wrong UI evidence transaction')
    require(all(proof['checks'].get(k) is True for k in ('all_62_controls', 'receiver_move_rejected', 'receiver_restored', 'map_retirement', 'representative_controls')), 'Incomplete UI evidence')
    require(proof['guard_after']['ready'] is False and proof['guard_after']['actual']['dx'] != proof['guard_before']['actual']['dx'], 'Receiver rejection unproven')
    require(sha(ROOT/'level-editor/app/src/editor-viewport.ts') == proof['runtime']['level-editor/app/src/editor-viewport.ts'], 'Proven viewport changed')
    for path, digest in proof['evidence'].items():
        require(sha(ROOT/path) == digest, 'Browser evidence drift: '+path)
    snapshot = read(PROOF.parent/'source-snapshot.json')
    require(snapshot == proof['runtime'], 'Browser runtime snapshot drift')
    # A drift disclosure records limits; it does not authorize skipping a guard.
    for path, digest in snapshot.items():
        require(sha(ROOT/path) == digest, 'Proven runtime changed: '+path)
    stage = read(STAGE/'transaction.json')
    for pin in stage['approvals'] + stage['browser_proofs'] + [p for r in stage['receiver_source_guards'] for p in r.values()]:
        io.core.verify_pin(pin)
    for row in stage['files']:
        io.core.verify_pin(row['staged'])
    return stage

def prepare(folder):
    stage = guards()
    before = read(LIB/'mission-states/index.json')
    before_sha = sha(LIB/'mission-states/index.json')
    merged = merge_catalog(before, stage['catalog_additions'])
    folder.mkdir(exist_ok=False)
    subprocess.run(['node', str(Path(__file__).with_name('restart58_verify_current_receivers.mjs')), str(folder/'source-verification.json')], cwd=ROOT, check=True)
    rows = []
    for row in stage['files']:
        relative = Path(row['destination'])
        if str(relative) == 'mission-states/index.json':
            continue
        require(relative.is_relative_to('mission-states/croisement02') and '..' not in relative.parts, 'Unexpected target')
        target = LIB/relative
        require(target.parent.resolve() == target.parent and not target.is_symlink(), 'Symlink target')
        old, new = sha(target), row['staged']['sha256']
        require(old in (None, new), 'Conflicting immutable resource')
        if old != new:
            rows.append({'source': row['staged']['path'], 'target': str(target), 'old_sha256': old, 'new_sha256': new})
    dump(folder/'index.json', merged)
    rows.append({'source': str(folder/'index.json'), 'target': str(LIB/'mission-states/index.json'), 'old_sha256': before_sha, 'new_sha256': sha(folder/'index.json')})
    require(sha(LIB/'mission-states/index.json') == before_sha, 'Catalog drift during rebase')
    plan = {'status': 'PREPARED_NOT_APPLIED', 'records': rows, 'catalog_before': before, 'additions': stage['catalog_additions'], 'browser': io.pin(PROOF), 'stage': io.pin(STAGE/'transaction.json'), 'map_guard': io.pin(LIB/'scenes/croisement02.rhlos-map.json'), 'executor': io.pin(Path(__file__).resolve()), 'source_verifier': io.pin(Path(__file__).with_name('restart58_verify_current_receivers.mjs')), 'scope': '62 additive catalog entries; immutable reviewed resources; catalog switched last. Unrelated entries/order/metadata preserved.', 'limitations': read(PROOF)['limitations']}
    dump(folder/'plan.json', plan)
    dump(folder/'dry-run.json', {'status': 'PASS_NOT_APPLIED', 'plan': io.pin(folder/'plan.json'), 'targets': len(rows), 'unrelated_entries_preserved': len(before['entries'])})
    return plan

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--root-reviewed', action='store_true')
    parser.add_argument('--readers-quiescent', action='store_true')
    args = parser.parse_args()
    folder = args.output.resolve()
    require(folder.parent == RUNS, 'Output outside owned directory')
    require(not args.apply or (args.root_reviewed and args.readers_quiescent), 'Root review and quiescent readers required')
    RUNS.mkdir(exist_ok=True)
    with io.core.locked():
        plan = prepare(folder)
        if args.apply:
            rows = plan['records']
            digest = sha(folder/'plan.json')
            reserve = sum(Path(r['source']).stat().st_size + (Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows)
            require(shutil.disk_usage(RUNS).free > reserve + 2**30, 'Insufficient rollback reserve')
            def precommit():
                guards()
                require(sha(Path(__file__)) == SELF_SHA and sha(folder/'plan.json') == digest, 'Executor or plan drift')
                io.core.verify_pin(plan['map_guard'])
                io.core.verify_pin(plan['source_verifier'])
                for r in rows:
                    require(sha(Path(r['source'])) == r['new_sha256'], 'Payload drift')
            precommit()
            for r in rows:
                Path(r['target']).parent.mkdir(parents=True, exist_ok=True)
            def postcheck():
                io.verify(rows, 'new_sha256')
                require(read(LIB/'mission-states/index.json') == merge_catalog(plan['catalog_before'], plan['additions']), 'Catalog merge mismatch')
                io.core.verify_pin(plan['map_guard'])
            io.commit(rows, folder/'transaction', digest, precommit, postcheck)
    print(json.dumps({'status': 'INSTALLED_LIVE_BROWSER_PENDING' if args.apply else 'PREPARED_NOT_APPLIED', 'output': str(folder)}))

if __name__ == '__main__':
    main()
