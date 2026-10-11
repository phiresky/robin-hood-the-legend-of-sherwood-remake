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
AUTHORITY = BASE / 'restart70-hole-mound-scoped-authority-v2/authority-proposal.json'
AUTHORITY_SHA = '736437ec7364c9cf5f65b8aff804076173a54c9dc786167276ddb8f0cb30a4d8'
SCOPED_PATHS = frozenset(('level-editor/shared/src/compile-asset-gameplay.ts', 'level-editor/shared/src/wall-spline-gameplay.ts', 'level-editor/shared/test-fixtures/wall-spline.ts'))
SELF_SHA = sha(Path(__file__))

def read(path):
    return json.loads(path.read_text())

def merge_catalog(current, additions):
    ids = [r['id'] for r in current['entries']]
    require(len(ids) == len(set(ids)), 'Duplicate existing catalog ids')
    require(len(additions) == 62 and len({r['id'] for r in additions}) == 62, 'Wrong additions')
    require(not set(ids).intersection(r['id'] for r in additions), 'Already installed or conflicting entries')
    return {**current, 'entries': [*current['entries'], *additions]}

def scoped_runtime(receipt_pin, snapshot):
    if receipt_pin is None:
        return {}
    receipt_path = Path(receipt_pin['path'])
    require(receipt_path.parent == AUTHORITY.parent and sha(receipt_path) == receipt_pin['sha256'], 'Root scope receipt drift')
    require(sha(AUTHORITY) == AUTHORITY_SHA, 'Scoped authority drift')
    receipt, authority = read(receipt_path), read(AUTHORITY)
    require(receipt['status'] == 'ROOT_APPROVED_SCOPED_STATE_PUBLICATION', 'Root scope approval missing')
    require(receipt['authority_sha256'] == AUTHORITY_SHA and receipt['transaction_sha256'] == STAGE_SHA and receipt['browser_sha256'] == PROOF_SHA, 'Root scope receipt mismatch')
    require(receipt.get('postinstall_ui_required') is True, 'Post-install UI requirement missing')
    require(authority['transaction_sha256'] == STAGE_SHA and authority['browser']['sha256'] == PROOF_SHA, 'Authority transaction mismatch')
    rows = authority['allowed_drift']
    require(len(rows) == 3 and {r['path'] for r in rows} == SCOPED_PATHS, 'Unexpected scoped runtime exclusions')
    for row in rows:
        require(snapshot[row['path']] == row['tested_sha256'], 'Scoped tested source mismatch')
        require(sha(ROOT/row['path']) == row['current_sha256'], 'Scoped current source drift: '+row['path'])
        require(sha(ROOT/row['diff']['path']) == row['diff']['sha256'], 'Scope diff drift')
    for pin in authority['dependency_evidence']:
        require(sha(ROOT/pin['path']) == pin['sha256'], 'Scope dependency evidence drift: '+pin['path'])
    return {r['path']: r['current_sha256'] for r in rows}

def guards(receipt_pin=None):
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
    # Only an exact root-reviewed scope receipt can accept these fixed alternate hashes.
    scoped = scoped_runtime(receipt_pin, snapshot)
    for path, digest in snapshot.items():
        require(sha(ROOT/path) == scoped.get(path, digest), 'Proven runtime changed: '+path)
    stage = read(STAGE/'transaction.json')
    for pin in stage['approvals'] + stage['browser_proofs'] + [p for r in stage['receiver_source_guards'] for p in r.values()]:
        io.core.verify_pin(pin)
    for row in stage['files']:
        io.core.verify_pin(row['staged'])
    return stage

def prepare(folder, receipt_pin=None):
    stage = guards(receipt_pin)
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
    if receipt_pin is not None:
        plan['scoped_runtime_authority'] = io.pin(AUTHORITY)
        plan['root_scope_receipt'] = receipt_pin
        plan['limitations'] += ['Scoped UI publication only; current gameplay compiler not certified. Canonical post-install UI check required.']
    dump(folder/'plan.json', plan)
    dump(folder/'dry-run.json', {'status': 'PASS_NOT_APPLIED', 'plan': io.pin(folder/'plan.json'), 'targets': len(rows), 'unrelated_entries_preserved': len(before['entries'])})
    return plan

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--root-reviewed', action='store_true')
    parser.add_argument('--readers-quiescent', action='store_true')
    parser.add_argument('--scope-receipt', type=Path)
    parser.add_argument('--scope-receipt-sha256')
    args = parser.parse_args()
    require(bool(args.scope_receipt) == bool(args.scope_receipt_sha256), 'Scope receipt and digest must be supplied together')
    receipt_pin = {'path': str(args.scope_receipt.resolve()), 'sha256': args.scope_receipt_sha256} if args.scope_receipt else None
    folder = args.output.resolve()
    require(folder.parent == RUNS, 'Output outside owned directory')
    require(not args.apply or (args.root_reviewed and args.readers_quiescent), 'Root review and quiescent readers required')
    RUNS.mkdir(exist_ok=True)
    with io.core.locked():
        plan = prepare(folder, receipt_pin)
        if args.apply:
            rows = plan['records']
            digest = sha(folder/'plan.json')
            reserve = sum(Path(r['source']).stat().st_size + (Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows)
            require(shutil.disk_usage(RUNS).free > reserve + 2**30, 'Insufficient rollback reserve')
            def precommit():
                guards(receipt_pin)
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
