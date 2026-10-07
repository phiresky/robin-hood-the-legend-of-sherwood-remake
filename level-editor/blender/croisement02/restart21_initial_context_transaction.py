"""Guarded eight-context transaction. Default is read-only; root supplies execution gate."""
import argparse
import fcntl
import hashlib
import json
from pathlib import Path
import restart17_initial_context_dryrun as proposal
import restart16_state_transaction as io

ROOT = proposal.ROOT
BASE = proposal.BASE
LIB = proposal.LIB
INDEX = proposal.INDEX
PROPOSAL = BASE / 'restart17-initial-context-publication-v3'
PROOF = BASE / 'restart20-initial-context-browser-v6'
OUTPUT = BASE / 'restart21-initial-context-transaction-v2'
PROPOSAL_SHA = '8d44a7e6d5d2d8f5c1e6514acaed2de4f4a9b61adb1f2d36f4cf96f04d3df0f4'
PROOF_SHA = '3ff86384a4b97ef02404632ad77ec1f1eb61ccc8eac81c4c53654556a88a5aab'
require, sha, read, safe = proposal.require, proposal.sha, proposal.read, proposal.safe


def pin(path):
    return {'path': str(path.relative_to(ROOT)), 'sha256': sha(path)}


def evidence(row):
    path = safe(ROOT, row['path'])
    require(sha(path) == row['sha256'], 'Evidence changed: ' + row['path'])
    return read(path)


def immutable_json(path, value):
    data = (json.dumps(value, indent=2) + '\n').encode()
    io.atomic_absent(path, data, hashlib.sha256(data).hexdigest())


def verify_checks(checks, ids):
    require(len(checks) == 68 and all(c['pass'] is True for c in checks), 'Browser checks failed')
    names = {c['name'] for c in checks}
    for identifier in ids:
        for suffix in ['exact contract', 'native selection', 'native initial reviewed CPU bytes',
                       'native applied reviewed CPU bytes', 'native exact reset pixels', 'initial', 'applied', 'reset']:
            require(identifier + ' ' + suffix in names, 'Missing browser assertion')
    require('Map-only removes all state roots' in names, 'Map-only not checked')


def prepare():
    require(sha(PROPOSAL / 'plan.json') == PROPOSAL_SHA, 'Proposal differs')
    require(sha(PROOF / 'browser/verification.json') == PROOF_SHA, 'Final browser proof differs')
    original = read(PROPOSAL / 'plan.json')
    proof = read(PROOF / 'browser/verification.json')
    inputs = read(PROOF / 'inputs.json')
    require(proof['inputs_sha256'] == sha(PROOF / 'inputs.json'), 'Proof inputs differ')
    require(proof['status'] == 'PASS_STAGED_EIGHT_INITIAL_CONTEXTS', 'Browser not passed')
    paths = ['inputs.json', 'runtime-baseline.json', 'preparation.json', 'browser/verification.json',
             'browser/remaining-seven-states/verification.json', 'browser/normal-http-resources.json',
             'browser/served-pins.json', 'browser/live-runtime-drift.json', 'browser/live-drift.json',
             'browser/process-final.json', 'browser/staged-catalog.json']
    for identifier in proof['selected_entry_ids']:
        paths += ['browser/remaining-seven-states/' + identifier + '-terminal.json']
    plan = {'schema': 'initial-context-transaction.v1', 'status': 'PREPARED_ROOT_GATE_REQUIRED',
            'proposal': pin(PROPOSAL / 'plan.json'), 'proof': pin(PROOF / 'browser/verification.json'),
            'evidence': [pin(PROOF / p) for p in paths],
            'recipe': pin(Path(__file__).resolve()),
            'helper_pins': [pin(Path(proposal.__file__).resolve()), pin(Path(io.__file__).resolve())],
            'baseline_index': original['baseline_index'], 'proposed_index': original['proposed_index'],
            'records': original['records'], 'runtime_files': read(PROOF / 'runtime-baseline.json')['source_files'],
            'input_files': inputs['files'], 'runtime_deltas': read(PROOF / 'preparation.json')['runtime_changes_since_v3'],
            'shared_catalog_delta': read(PROOF / 'preparation.json').get('shared_catalog_delta'),
            'scope': original['scope'], 'old_contracts_preserved': True,
            'after_switch': 'Root must run installed normal-HTTP verification before final acceptance.'}
    OUTPUT.mkdir(exist_ok=False)
    immutable_json(OUTPUT / 'plan.json', plan)
    return plan


def verify(plan):
    require(plan['proposal']['sha256'] == PROPOSAL_SHA and plan['proof']['sha256'] == PROOF_SHA, 'Unreviewed proposal/proof')
    original = evidence(plan['proposal'])
    proof = evidence(plan['proof'])
    for row in [plan['recipe'], *plan['helper_pins']]:
        require(sha(safe(ROOT, row['path'])) == row['sha256'], 'Transaction code changed')
    documents = {r['path']: evidence(r) for r in plan['evidence']}
    require(len(documents) == 19, 'Proof evidence membership differs')
    def doc(name):
        return documents[str((PROOF / name).relative_to(ROOT))]
    require(proof['status'] == 'PASS_STAGED_EIGHT_INITIAL_CONTEXTS', 'Browser not passed')
    require(proof['inputs_sha256'] == sha(PROOF / 'inputs.json'), 'Browser input binding differs')
    require(plan['input_files'] == doc('inputs.json')['files'], 'Pinned input membership differs')
    require(plan['runtime_files'] == doc('runtime-baseline.json')['source_files'], 'Runtime pin inventory differs')
    require(set(proposal.runtime_paths(plan['runtime_files'])) == set(plan['runtime_files']), 'New runtime module not covered by browser proof')
    require(plan['runtime_deltas'] == doc('preparation.json')['runtime_changes_since_v3'], 'Runtime delta review scope differs')
    require(plan['shared_catalog_delta'] == doc('preparation.json').get('shared_catalog_delta'), 'Shared catalog delta review scope differs')
    require(not doc('browser/live-runtime-drift.json')['changed'] and not doc('browser/live-drift.json')['changed'], 'Browser drift')
    require(doc('browser/process-final.json') == {'exitCode': 0, 'signalCode': None, 'serverClosed': True}, 'Browser did not close cleanly')
    require(plan['records'] == original['records'], 'Replacement file membership differs')
    require(plan['baseline_index'] == original['baseline_index'] and plan['proposed_index'] == original['proposed_index'], 'Index identity differs')
    before, after = evidence(plan['baseline_index']), evidence(plan['proposed_index'])
    proposal.validate_index_delta(before, after, plan['records'])
    require(set(proof['selected_entry_ids']) == {r['id'] for r in plan['records']}, 'Wrong browser entries')
    verify_checks(doc('browser/remaining-seven-states/verification.json')['checks'], proof['selected_entry_ids'])
    for row in original['candidate_evidence']:
        evidence(row)
    reviewed = read(proposal.CANDIDATE / 'manifest.json')
    reviewed_by_id = {r['id']: r for r in reviewed['records']}
    for row in plan['records']:
        proposal.validate_candidate_record(row, reviewed_by_id[row['id']])
        source = safe(ROOT, row['source'])
        require(sha(source) == row['sha256'], 'Source contract drift')
        require(sha(safe(LIB, row['baseline_path'])) == row['baseline_sha256'], 'Old contract drift')
        proposal.validate_contract_delta(read(safe(LIB, row['baseline_path'])), read(source), row['added_context_ids'])
        target = safe(LIB, row['destination'])
        if target.exists():
            require(sha(target) == row['sha256'], 'Conflicting immutable contract')
    for path, digest in plan['input_files'].items():
        require(sha(safe(ROOT, path)) == digest, 'Browser-tested input changed: ' + path)
    for row in doc('browser/normal-http-resources.json'):
        if row.get('virtualProductionListing'):
            require(row['path'] == 'scenes/index.json', 'Unexpected virtual resource')
            continue
        overlay = {INDEX: plan['proposed_index']['path'], **{r['destination']: r['source'] for r in plan['records']}}
        path = safe(ROOT, overlay[row['path']]) if row['path'] in overlay else safe(LIB, row['path'])
        require(sha(path) == row['sha256'], 'Successful HTTP resource changed: ' + row['path'])
    require(sha(LIB / INDEX) == plan['baseline_index']['sha256'], 'Catalog no longer baseline41')
    return {'status': 'PASS_TRANSACTION_DRY_RUN_ROOT_GATE_PENDING', 'entries': 41, 'unchanged_entries': 33,
            'new_immutable_contracts': 8, 'browser_checks': 68, 'input_pins': len(plan['input_files']),
            'runtime_files': len(plan['runtime_files']), 'runtime_deltas': plan['runtime_deltas'], 'library_modified': False}


def verify_gate(path, digest, plan, plan_sha):
    require(path is not None and digest is not None, 'Explicit root gate and digest required')
    require(sha(path) == digest, 'Root gate changed')
    gate = read(path)
    require(gate.get('status') == 'PASS_ROOT_READY_FOR_INITIAL_CONTEXT_PUBLICATION', 'Root publication review missing')
    require(gate.get('transaction_plan_sha256') == plan_sha and gate.get('browser_proof_sha256') == PROOF_SHA, 'Gate binds other proposal')
    require(gate.get('reviewed_runtime_deltas') == plan['runtime_deltas'], 'Current runtime deltas not reviewed')
    require(gate.get('reviewed_shared_catalog_delta') == plan['shared_catalog_delta'], 'Current shared catalog delta not reviewed')
    require(gate.get('scope') == 'raw-pre-script-initial-context' and gate.get('publication_authorized') is True, 'Publication scope/authorization missing')
    return gate


def rollback(index, baseline, old, new, receipt, plan_sha):
    require(receipt.get('plan_sha256') == plan_sha and receipt.get('baseline_index_sha256') == old and receipt.get('index_sha256') == new, 'Rollback receipt not owned')
    require(receipt.get('status') in {'TRANSACTION_PREPARED', 'INSTALLED_PENDING_NORMAL_HTTP_PROOF', 'INDEX_SWITCHED_PENDING_RECOVERY'}, 'Rollback receipt status invalid')
    require(sha(index) == new, 'Rollback refuses another writer index')
    io.switch_index(index, baseline, new, old)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    mode = ap.add_mutually_exclusive_group()
    for flag in ['prepare', 'execute', 'rollback']:
        mode.add_argument('--' + flag, action='store_true')
    ap.add_argument('--plan-sha'); ap.add_argument('--gate', type=Path); ap.add_argument('--gate-sha')
    ap.add_argument('--receipt', type=Path); ap.add_argument('--installation', type=Path); ap.add_argument('--installation-sha')
    args = ap.parse_args()
    if args.prepare:
        plan = prepare()
        report = verify(plan)
        immutable_json(OUTPUT / 'dry-run.json', report)
        print(json.dumps({**report, 'plan_sha256': sha(OUTPUT / 'plan.json')})); return
    require(args.plan_sha and sha(OUTPUT / 'plan.json') == args.plan_sha, 'Exact transaction plan hash required')
    plan = read(OUTPUT / 'plan.json')
    if not (args.execute or args.rollback):
        report = verify(plan)
        if args.gate:
            verify_gate(args.gate, args.gate_sha, plan, args.plan_sha)
            report['status'] = 'PASS_TRANSACTION_AND_ROOT_GATE_DRY_RUN'
        print(json.dumps(report)); return
    require(args.receipt is not None and not args.receipt.exists(), 'Fresh receipt required')
    with (OUTPUT / 'transaction.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(sha(OUTPUT / 'plan.json') == args.plan_sha, 'Plan changed before lock')
        old, new = plan['baseline_index']['sha256'], plan['proposed_index']['sha256']
        if args.rollback:
            require(args.installation and args.installation_sha == sha(args.installation), 'Exact owned installation receipt required')
            rollback(LIB / INDEX, safe(ROOT, plan['baseline_index']['path']).read_bytes(), old, new, read(args.installation), args.plan_sha)
            immutable_json(args.receipt, {'status': 'ROLLED_BACK_INDEX_ONLY', 'restored_sha256': old, 'immutable_contracts_retained': True})
            print('ROLLED_BACK_INDEX_ONLY'); return
        verify(plan); verify_gate(args.gate, args.gate_sha, plan, args.plan_sha)
        intent = {'status': 'TRANSACTION_PREPARED', 'plan_sha256': args.plan_sha, 'index_sha256': new,
                  'baseline_index_sha256': old, 'gate_sha256': args.gate_sha, 'created': []}
        intent_path = args.receipt.with_name(args.receipt.name + '.intent.json')
        require(not intent_path.exists(), 'Fresh intent required')
        immutable_json(intent_path, intent)
        try:
            for row in plan['records']:
                if io.atomic_absent(safe(LIB, row['destination']), safe(ROOT, row['source']).read_bytes(), row['sha256']):
                    intent['created'].append(row['destination'])
            verify(plan); verify_gate(args.gate, args.gate_sha, plan, args.plan_sha)
            for row in plan['records']:
                require(sha(safe(LIB, row['destination'])) == row['sha256'], 'New immutable contract changed')
            io.switch_index(LIB / INDEX, safe(ROOT, plan['proposed_index']['path']).read_bytes(), old, new)
            require(sha(LIB / INDEX) == new, 'Post-switch catalog mismatch')
        except Exception as error:
            status = 'INDEX_SWITCHED_PENDING_RECOVERY' if sha(LIB / INDEX) == new else 'FAILED_BEFORE_SWITCH_OR_EXTERNAL_INDEX'
            immutable_json(args.receipt, {**intent, 'status': status, 'error': str(error), 'observed_index_sha256': sha(LIB / INDEX)})
            raise
        immutable_json(args.receipt, {**intent, 'status': 'INSTALLED_PENDING_NORMAL_HTTP_PROOF', 'old_contracts_preserved': True})
        print('INSTALLED_PENDING_NORMAL_HTTP_PROOF')

if __name__ == '__main__':
    main()
