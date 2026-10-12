"""Root-only, locked hall catalog publication; private validation never reads live assets."""
import argparse
import json
import shutil
from pathlib import Path
import restart44_static_transaction as io
ROOT, LIB, BASE = io.ROOT, io.LIB, io.WORK
STAGE = BASE / 'restart72-hall-normal-catalog-stage-v1'
RUNS = BASE / 'restart73-hall-catalog-transaction-v1'
STAGE_SHA = '4c7a7148d5704b66dd964e0d25df5c01ab8741f900b05b1563c5d038ec54be8e'
APPROVAL = ROOT / 'level-editor/work/active/croisement02/restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json'
APPROVAL_SHA = 'd435a76b08eb3ac616f1ae2ffcdf19a7875969e515dd01f65de1871ca886ef1d'
HELPER_SHA = '9cb6c1e3f87c4a8c2f3f3e28a02aa28448914188b378b761508e0f14f3186ce9'
UI_SCRIPT = ROOT / 'level-editor/blender/york/restart72_hall_catalog_ui_check.js'
UI_SHA = 'a961607a6a61325ba7865271d32ca5f73ef98c9b5808a7ba45eb5bf0836a4d13'
MODEL_SHA = '3edab8b9da5fd995205224e956c5a76d716eaaa49d7d3b3380df04e0dbf99eba'
sha, require, dump = io.sha, io.require, io.dump
SELF_SHA = sha(Path(__file__))
read = lambda p: json.loads(p.read_text())


def stage_plan():
    require(sha(STAGE / 'plan.operational.json') == STAGE_SHA, 'Stage drift')
    require(sha(Path(io.__file__)) == HELPER_SHA, 'Transaction helper drift')
    require(sha(APPROVAL) == APPROVAL_SHA, 'Exact six-scope approval required')
    decisions = [d for d in read(APPROVAL)['decisions'] if d['asset_id'] == 'york-castle-great-hall--animation000-flames-reflections']
    require(len(decisions) == 1 and decisions[0]['decision'] == 'approved' and decisions[0]['scope'] == 'geometry + source appearance' and decisions[0]['model_sha256'] == '97190d43ae97e3abc3afeecce30b239ce44e291e9d1773ebdabe249ca2a5b81d', 'Hall approval scope differs')
    stage = read(STAGE / 'plan.operational.json')
    seen = set()
    for row in stage['records']:
        rel, source = Path(row['relative']), Path(row['source'])
        require(not rel.is_absolute() and '..' not in rel.parts and (rel.is_relative_to('mission-states/york') or str(rel) == 'mission-states/index.json'), 'Out-of-scope resource')
        require(str(rel) not in seen, 'Duplicate resource'); seen.add(str(rel))
        require(source == STAGE / 'library' / rel and source.resolve() == source and not source.is_symlink(), 'Stage path escaped')
        require(sha(source) == row['sha256'], 'Stage resource drift')
    entry = stage['entry']
    require(entry['id'] == 'york-hall000-reviewed-loop' and entry['kind'] == 'native-loop', 'Wrong hall catalog entry')
    contract_path = STAGE / 'library' / entry['contract']['path']
    require(sha(contract_path) == entry['contract']['sha256'], 'Gated contract drift')
    c = read(contract_path)
    require(c['physical']['targets'][0]['source']['model_sha256'] == MODEL_SHA, 'Approved flame bytes differ')
    require(len(c['cover_visibility']) == 1 and c['cover_visibility'][0]['source']['index'] == 2 and c['cover_visibility'][0]['visible_when'] == 'applied', 'Independent patch002 visibility missing')
    require({tuple(v['applied']) for v in c['native']['cover_artwork']['variants']} == {(), ('patch-001',), ('patch-002',), ('patch-001', 'patch-002')}, 'Four source backgrounds missing')
    return stage, c


def merged_catalog(before, entry):
    require(not any(r['id'] == entry['id'] for r in before['entries']), 'Hall entry already installed; reconcile receipt')
    return {**before, 'entries': [*before['entries'], entry]}


def required_captures():
    return ({f'york-hall-ui-{a}{b}-physical-view{i}' for a in (0, 1) for b in (0, 1) for i in (0, 3)} |
            {f'york-hall-ui-{a}{b}-source-window' for a in (0, 1) for b in (0, 1)} |
            {f'york-hall-ui-loop-frame{i}' for i in range(6)} | {'york-hall-ui-reset', 'york-hall-ui-retired'})


def proof(path, digest, stage):
    require(path is not None and digest is not None and sha(path) == digest, 'Exact reviewed normal UI proof required')
    d = read(path)
    require(d.get('status') == 'PASS' and d.get('entry_id') == stage['entry']['id'] and d.get('contract_sha256') == stage['entry']['contract']['sha256'], 'Wrong normal UI proof scope')
    require(d.get('visual_review') is True and d.get('approval_sha256') == APPROVAL_SHA, 'Visual review and exact six-scope receipt required')
    require(sha(UI_SCRIPT) == UI_SHA and d.get('ui_script_sha256') == UI_SHA, 'Normal UI script changed')
    require(bool(d.get('evidence')), 'Missing immutable browser evidence')
    for name, expected in d['evidence'].items():
        require(sha(Path(name)) == expected, 'Browser evidence drift')
    raw_pin = d['ui_evidence']
    require(d['evidence'].get(raw_pin['path']) == raw_pin['sha256'], 'UI result not pinned in evidence')
    raw = read(Path(raw_pin['path']))
    require(raw.get('status') == 'EXECUTED_NORMAL_EDITOR_PENDING_VISUAL_REVIEW' and raw.get('entry_id') == stage['entry']['id'] and raw.get('contract_sha256') == stage['entry']['contract']['sha256'], 'Wrong executable UI result')
    require(raw.get('checks') and all(r.get('pass') is True for r in raw['checks']), 'Normal UI assertion failed')
    require(required_captures() <= set(raw.get('captures', [])), 'Incomplete normal UI captures')
    require(required_captures() <= set(d.get('capture_evidence', {})), 'Missing screenshot pins')
    for label, pin in d['capture_evidence'].items():
        require(d['evidence'].get(pin['path']) == pin['sha256'], 'Screenshot not pinned in evidence: ' + label)
    return io.pin(path)


def validate_installed_hall(contract):
    """Root-only canonical reader, called under the publication lock after reader release."""
    document = read(LIB / 'scenes/york.rhlos-map.json')
    for binding in contract['native']['cover_artwork']['bindings']:
        p = binding['placement']
        group = next((g for g in document['placements'] if g['id'] == p['group_id']), None)
        asset = next((a for a in document['assetSources'] if a['id'] == p['asset_id']), None)
        require(group is not None and asset is not None, 'Installed hall placement absent')
        require(group['transform'] == p['transform'] and group['patches'][p['asset_id']][p['appearance_id']] == p['patch_id'], 'Actual roof binding changed')
        for key in ('model', 'descriptor'):
            require(asset[key + '_sha256'] == p[key + '_sha256'] and sha(LIB / asset[key]) == p[key + '_sha256'], 'Installed hall bytes changed')


def prepare(folder, browser, browser_sha):
    stage, contract = stage_plan()
    browser_pin = proof(browser, browser_sha, stage)
    validate_installed_hall(contract)
    rows = []
    for r in stage['records']:
        if r['relative'] == 'mission-states/index.json': continue
        target = LIB / r['relative']
        require(target.parent.resolve() == target.parent and not target.is_symlink(), 'Symlink publication path')
        old = sha(target)
        require(old in (None, r['sha256']), 'Conflicting immutable hall resource')
        if old != r['sha256']:
            rows.append({'source': r['source'], 'target': str(target), 'old_sha256': old, 'new_sha256': r['sha256']})
    index = LIB / 'mission-states/index.json'
    before, old_index = read(index), sha(index)
    after = merged_catalog(before, stage['entry'])
    folder.mkdir(exist_ok=False)
    # Compact only whitespace; latest unrelated rows/order/metadata retained exactly.
    (folder / 'index.json').write_text(json.dumps(after, separators=(',', ':'), ensure_ascii=False) + '\n')
    rows.append({'source': str(folder / 'index.json'), 'target': str(index), 'old_sha256': old_index, 'new_sha256': sha(folder / 'index.json')})
    require(sha(index) == old_index, 'Catalog drift during prepare')
    result = {'status': 'PREPARED_NOT_APPLIED', 'records': rows, 'catalog_before': before, 'entry': stage['entry'], 'stage': io.pin(STAGE / 'plan.operational.json'), 'approval': io.pin(APPROVAL), 'browser': browser_pin, 'executor': io.pin(Path(__file__).resolve()), 'helper': io.pin(Path(io.__file__).resolve())}
    dump(folder / 'plan.json', result)
    dump(folder / 'dry-run.json', {'status': 'PASS_NOT_APPLIED', 'plan': io.pin(folder / 'plan.json')})
    return result


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--private-check', action='store_true')
    p.add_argument('--output', type=Path)
    p.add_argument('--apply', action='store_true')
    p.add_argument('--rollback', action='store_true')
    p.add_argument('--root-reviewed', action='store_true')
    p.add_argument('--readers-quiescent', action='store_true')
    p.add_argument('--browser-proof', type=Path)
    p.add_argument('--browser-sha256')
    a = p.parse_args()
    if a.private_check:
        require(not a.apply and not a.rollback and a.output is None, 'Private check cannot mutate publication')
        stage, _ = stage_plan()
        before = {'version': 1, 'metadata': {'keep': True}, 'entries': [{'id': 'new-other-session', 'extra': [1, 2]}]}
        after = merged_catalog(before, stage['entry'])
        require(after['entries'][:-1] == before['entries'] and after['metadata'] == before['metadata'], 'Rebase preservation failed')
        print(json.dumps({'status': 'PASS_PRIVATE_STAGE_ONLY', 'resources': len(stage['records']), 'canonical_reads': False, 'canonical_writes': False, 'browser_proof': 'PENDING_REQUIRED_BEFORE_PREPARE_OR_APPLY'})); return
    require(a.root_reviewed and a.readers_quiescent, 'Root ownership and quiescent readers required even for canonical preparation')
    require(a.output is not None, 'Output required')
    folder = a.output.resolve()
    require(folder.parent == RUNS, 'Output outside owned directory'); RUNS.mkdir(exist_ok=True)
    with io.core.locked():
        if a.rollback:
            require(not a.apply, 'Conflicting actions')
            plan, dry, journal = read(folder / 'plan.json'), read(folder / 'dry-run.json'), read(folder / 'transaction/journal.json')
            require(sha(folder / 'plan.json') == dry['plan']['sha256'] == journal['plan_sha256'], 'Rollback plan drift')
            require([(r['target'], r['old_sha256'], r['new_sha256']) for r in plan['records']] == [(r['target'], r['old_sha256'], r['new_sha256']) for r in journal['files']], 'Rollback scope drift')
            require(all(Path(r['target']).is_relative_to(LIB / 'mission-states/york') or Path(r['target']) == LIB / 'mission-states/index.json' for r in plan['records']), 'Rollback escaped scope')
            io.core.restore(journal['files'], folder / 'transaction', journal); return
        plan = prepare(folder, a.browser_proof, a.browser_sha256)
        rows, digest = plan['records'], sha(folder / 'plan.json')
        if a.apply:
            reserve = sum(Path(r['source']).stat().st_size + (Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows)
            require(shutil.disk_usage(RUNS).free > reserve + 2**30, 'Insufficient rollback reserve')
            def check():
                stage, c = stage_plan(); proof(a.browser_proof, a.browser_sha256, stage); validate_installed_hall(c)
                require(sha(Path(__file__)) == SELF_SHA and sha(folder / 'plan.json') == digest, 'Executor or plan drift')
                for r in rows:
                    require(sha(Path(r['source'])) == r['new_sha256'], 'Payload drift')
                    t = Path(r['target']); require(t.parent.resolve() == t.parent and not t.is_symlink(), 'Target path changed')
            check()
            for r in rows: Path(r['target']).parent.mkdir(parents=True, exist_ok=True)
            def postcheck():
                io.verify(rows, 'new_sha256')
                require(read(LIB / 'mission-states/index.json') == merged_catalog(plan['catalog_before'], plan['entry']), 'Live unrelated catalog drift')
                stage, c = stage_plan(); validate_installed_hall(c)
                for r in stage['records']:
                    if r['relative'] != 'mission-states/index.json': require(sha(LIB / r['relative']) == r['sha256'], 'Installed resource drift')
            io.commit(rows, folder / 'transaction', digest, check, postcheck)
        print(json.dumps({'status': 'INSTALLED_LIVE_BROWSER_PENDING' if a.apply else 'PREPARED_NOT_APPLIED', 'output': str(folder)}))

if __name__ == '__main__': main()
