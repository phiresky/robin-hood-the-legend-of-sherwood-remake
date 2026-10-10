"""Root-run York publication, including reversible obsolete derivative removal.

Default preparation writes private evidence only. Apply requires reviewed browser
proof and quiescent readers; the shared publication lock covers preflight to commit.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'level-editor/library'
ASSETS = LIB / '3d-assets'
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
STAGE = WORK / 'restart43-approved-static-stage-v3'
RUNS = WORK / 'restart44-static-transaction-v1'
CORE = ROOT / 'level-editor/work/croisement02-refinement/restart25-approved-appearance-delta-v1/derivatives-v3/transaction.py'
spec = importlib.util.spec_from_file_location('york_publication_core', CORE)
core = importlib.util.module_from_spec(spec)
spec.loader.exec_module(core)
require, sha, dump = core.require, core.sha, core.durable_json
require(sha(CORE) == '1b79d5020ec58855514800d8d48b6196eefefedb19ea2ac0937245d8e609787f', 'Publisher core changed')
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from asset_index import write_asset_index, validate_asset_index


def pin(path):
    return {'path': str(path), 'sha256': sha(path)}


def verify(records, key):
    for row in records:
        require(sha(Path(row['target'])) == row[key], 'Target drift: ' + row['target'])


def commit(records, folder, plan_hash, precommit, postcheck, fail_after=None):
    """Same durable backup/rename protocol as the shared core, with deletion."""
    folder.mkdir(exist_ok=False)
    require(folder.stat().st_dev == Path(records[0]['target']).parent.stat().st_dev,
            'Backup and target must share filesystem')
    journal = {'plan_sha256': plan_hash, 'phase': 'PREPARING', 'files': records,
               'installed': [], 'atomicity': 'Per-file only; quiescent readers required'}
    dump(folder / 'journal.json', journal)
    for i, row in enumerate(records):
        target = Path(row['target'])
        require(sha(target) == row['old_sha256'], 'Drift before backup')
        row['old_mode'] = target.stat().st_mode & 0o777 if target.exists() else None
        row['backup'] = str(folder / f'{i:02d}.backup') if target.exists() else None
        row['incoming'] = str(folder / f'{i:02d}.incoming') if row['source'] else None
        if row['backup']:
            core.durable_copy(target, Path(row['backup']))
            require(sha(Path(row['backup'])) == row['old_sha256'], 'Backup mismatch')
            Path(row['backup']).chmod(0o444)
        if row['incoming']:
            core.durable_copy(Path(row['source']), Path(row['incoming']))
            require(sha(Path(row['incoming'])) == row['new_sha256'], 'Incoming mismatch')
    journal['phase'] = 'PREPARED'
    dump(folder / 'journal.json', journal)
    precommit()
    verify(records, 'old_sha256')
    try:
        for i, row in enumerate(records):
            target = Path(row['target'])
            require(sha(target) == row['old_sha256'], 'Drift immediately before mutation')
            journal.update(phase='INSTALLING', next_target=str(target))
            dump(folder / 'journal.json', journal)
            if row['source']:
                require(sha(Path(row['incoming'])) == row['new_sha256'], 'Incoming drift')
                os.replace(row['incoming'], target)
            else:
                require(row['new_sha256'] is None and row['old_sha256'] is not None,
                        'Invalid removal')
                target.unlink()
            core.fsync_dir(target.parent)
            journal['installed'].append(str(target))
            dump(folder / 'journal.json', journal)
            if fail_after == i + 1:
                raise RuntimeError('Injected failure')
        postcheck()
        journal['phase'] = 'INSTALLED_LIVE_BROWSER_PENDING'
        journal.pop('next_target', None)
        dump(folder / 'journal.json', journal)
    except BaseException:
        core.restore(records, folder, journal)
        raise
    return journal


def stage_plan():
    require(sha(STAGE / 'plan.json') == '31e7891d959d636ea462a98edd38121aafb51901b1190b1eaee70b023958fe22', 'Stage plan drift')
    plan = json.loads((STAGE / 'plan.json').read_text())
    ids = set(plan['asset_ids'])
    allowed = {str(ASSETS / 'york' / aid / name) for aid in ids for name in
               ['model.glb', 'asset.json', 'lossy.glb', 'preview.glb', 'lossy.glb.receipt.json', 'preview.glb.receipt.json']}
    allowed |= {str(LIB / 'scenes/york.rhlos-map.json'), str(ROOT / 'level-editor/refinement/catalogs/york.json')}
    rows = plan['records']
    require(len(rows) == 26 and {r['target'] for r in rows} == allowed, 'Unexpected transaction scope')
    for row in rows:
        target = core.path(row['target'])
        require(target.parent.resolve() == target.parent, 'Symlink target parent')
        if row['source']:
            require(sha(core.path(row['source'])) == row['new_sha256'], 'Stage payload drift')
        else:
            require(row['operation'] == 'remove_obsolete_derivative' and row['new_sha256'] is None, 'Invalid removal')
    verify(rows, 'old_sha256')
    for item in plan['protected'] + [plan['affected_index']]:
        core.verify_pin(item)
    return plan


def preserve(plan):
    old = json.loads((LIB / 'scenes/york.rhlos-map.json').read_text())
    new = json.loads((STAGE / 'york.rhlos-map.json').read_text())
    ids = set(plan['asset_ids'])
    require({k: v for k, v in old.items() if k != 'assetSources'} ==
            {k: v for k, v in new.items() if k != 'assetSources'}, 'Unrelated map change')
    a, b = ({r['id']: r for r in doc['assetSources']} for doc in [old, new])
    require(a.keys() == b.keys(), 'Map source membership change')
    require(all(row == b[key] for key, row in a.items() if key not in ids), 'Unrelated map reference change')
    old_catalog = json.loads((ROOT / 'level-editor/refinement/catalogs/york.json').read_text())
    new_catalog = json.loads((STAGE / 'york-catalog.json').read_text())
    require({k: v for k, v in old_catalog.items() if k != 'groups'} ==
            {k: v for k, v in new_catalog.items() if k != 'groups'}, 'Catalog metadata changed')
    old_groups, new_groups = ({r['id']: r for r in doc['groups']} for doc in [old_catalog, new_catalog])
    require(old_groups.keys() == new_groups.keys(), 'Catalog membership changed')
    require(all(row == new_groups[key] for key, row in old_groups.items() if key not in ids),
            'Unrelated named group changed')
    return {'placements': len(old['placements']), 'unrelated_map_references': len(a) - len(ids),
            'unrelated_catalog_groups': len(old_groups) - len(ids)}


def browser_evidence(file, digest, rows):
    require(file is not None and digest is not None and sha(file) == digest, 'Pinned browser proof required')
    evidence = json.loads(file.read_text())
    require(evidence.get('status') == 'PASS', 'Browser proof must pass')
    models = {Path(r['target']).parent.name: r['new_sha256'] for r in rows
              if Path(r['target']).name == 'model.glb'}
    require(evidence.get('approved_models') == models, 'Browser proof model scope mismatch')
    require(bool(evidence.get('evidence')), 'Browser proof has no evidence pins')
    for path, expected in evidence['evidence'].items():
        core.verify_pin({'path': path, 'sha256': expected})
    return pin(file)


def prepare(folder, browser=None, browser_hash=None):
    stage = stage_plan()
    preservation = preserve(stage)
    folder.mkdir(exist_ok=False)
    rows = json.loads(json.dumps(stage['records']))
    files = {str(Path(r['target']).relative_to(ASSETS)): Path(r['source']) if r['source'] else None
             for r in rows if Path(r['target']).is_relative_to(ASSETS)}
    ids = set(stage['asset_ids'])
    write_asset_index(ASSETS, target=folder / 'affected-index.json', files=files,
                      descriptors=[f'york/{aid}/asset.json' for aid in sorted(ids)])
    affected = json.loads((folder / 'affected-index.json').read_text())['assets']
    require({r['id'] for r in affected} == ids, 'Index generation scope mismatch')
    current = json.loads((ASSETS / 'index.json').read_text())
    index_pin = pin(ASSETS / 'index.json')
    changes = {r['id']: r for r in affected}
    require(ids <= {r['id'] for r in current['assets']}, 'Affected index entry absent')
    merged = {**current, 'assets': [changes.get(r['id'], r) for r in current['assets']]}
    validate_asset_index(ASSETS, merged, files=files)
    # Global catalogs must stay below the JavaScript string-size limit. Preserve
    # data and entry order while omitting insignificant pretty-print whitespace.
    with (folder / 'index.json').open('x') as stream:
        json.dump(merged, stream, separators=(',', ':'), ensure_ascii=False)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    rows.append({'source': str(folder / 'index.json'), 'target': str(ASSETS / 'index.json'),
                 'old_sha256': index_pin['sha256'], 'new_sha256': sha(folder / 'index.json'), 'operation': 'replace'})
    protected = [p for p in stage['protected'] if p['path'] not in {r['target'] for r in rows}]
    for entry in current['assets']:
        if entry['id'] not in ids:
            protected.append(pin(ASSETS / entry['descriptor']))
    document = json.loads((STAGE / 'york.rhlos-map.json').read_text())
    for ref in document['assetSources'] + document.get('sceneAssets', []):
        for resource in ref.get('resources', []):
            item = {'path': str(LIB / resource['path']), 'sha256': resource['sha256']}
            core.verify_pin(item)
            protected.append(item)
        if ref['id'] not in ids:
            for key in ['model', 'descriptor']:
                if key in ref:
                    item = {'path': str(LIB / ref[key]), 'sha256': ref[key + '_sha256']}
                    core.verify_pin(item)
                    protected.append(item)
    proof = browser_evidence(browser, browser_hash, rows) if browser else None
    require(sha(ASSETS / 'index.json') == index_pin['sha256'], 'Index drift during preparation')
    result = {'status': 'PREPARED_NOT_APPLIED', 'records': rows, 'protected': protected,
              'browser': proof, 'preservation': preservation,
              'evidence': [pin(Path(__file__)), pin(CORE), pin(STAGE / 'plan.json'),
                           pin(ROOT / 'level-editor/refinement/asset_index.py'),
                           pin(ROOT / 'level-editor/blender/york/restart43_validate_static_stage.mjs')],
              'index_policy': 'Only four source-of-truth rows regenerated; current unrelated rows, order and top-level fields preserved.',
              'limitations': 'Per-file atomicity, quiescent readers required. Fresh live browser verification after installation.'}
    dump(folder / 'plan.json', result)
    dump(folder / 'dry-run.json', {'status': 'PASS_NOT_APPLIED', 'plan': pin(folder / 'plan.json'),
                                  'targets': len(rows), 'removals': sum(r['source'] is None for r in rows),
                                  'browser_verified': proof is not None})
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--rollback', action='store_true')
    parser.add_argument('--root-reviewed', action='store_true')
    parser.add_argument('--readers-quiescent', action='store_true')
    parser.add_argument('--browser-proof', type=Path)
    parser.add_argument('--browser-sha256')
    parser.add_argument('--editor-proof', type=Path)
    parser.add_argument('--editor-sha256')
    args = parser.parse_args()
    folder = args.output.resolve()
    require(folder.parent == RUNS, 'Output outside owned transaction directory')
    RUNS.mkdir(exist_ok=True)
    with core.locked():
        if args.rollback:
            plan = json.loads((folder / 'plan.json').read_text())
            require(sha(folder / 'plan.json') == json.loads((folder / 'dry-run.json').read_text())['plan']['sha256'], 'Plan drift')
            journal = json.loads((folder / 'transaction/journal.json').read_text())
            require(journal['plan_sha256'] == sha(folder / 'plan.json'), 'Journal plan mismatch')
            require([(r['target'], r['old_sha256'], r['new_sha256']) for r in journal['files']] ==
                    [(r['target'], r['old_sha256'], r['new_sha256']) for r in plan['records']], 'Rollback scope mismatch')
            core.restore(journal['files'], folder / 'transaction', journal)
            return
        require(not args.apply or (args.root_reviewed and args.readers_quiescent and args.browser_proof),
                'Root review, quiescent readers and browser proof required')
        plan = prepare(folder, args.browser_proof, args.browser_sha256)
        if args.apply:
            rows = plan['records']
            require(args.editor_proof and args.editor_sha256,
                    'Full map/group/part/palette/reload browser proof required before publication')
            editor_pin = browser_evidence(args.editor_proof, args.editor_sha256, rows)
            editor = json.loads(args.editor_proof.read_text())
            require(editor.get('editor_checks') == {
                'map_load': True, 'group_selection': True, 'part_selection': True,
                'palette_insert': True, 'save_reload': True}, 'Incomplete editor integration proof')
            plan['editor_browser'] = editor_pin
            dump(folder / 'plan.json', plan)
            dry = json.loads((folder / 'dry-run.json').read_text())
            dry['plan'] = pin(folder / 'plan.json')
            dump(folder / 'dry-run.json', dry)
            plan_hash = sha(folder / 'plan.json')
            reserve = sum((Path(r['source']).stat().st_size if r['source'] else 0) +
                          (Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows)
            require(shutil.disk_usage(RUNS).free > reserve + 2**30, 'Insufficient rollback reserve')
            def precommit():
                stage_plan()
                require(sha(folder / 'plan.json') == plan_hash, 'Plan changed')
                for item in plan['protected'] + plan['evidence']:
                    core.verify_pin(item)
                browser_evidence(args.browser_proof, args.browser_sha256, rows)
                browser_evidence(args.editor_proof, args.editor_sha256, rows)
            def postcheck():
                verify(rows, 'new_sha256')
                for item in plan['protected']:
                    core.verify_pin(item)
                validate_asset_index(ASSETS, json.loads((ASSETS / 'index.json').read_text()))
                subprocess.run(['node', str(ROOT / 'level-editor/blender/york/restart43_validate_static_stage.mjs'),
                                str(LIB / 'scenes/york.rhlos-map.json'), str(LIB),
                                str(folder / 'strict-loader.json')], cwd=ROOT, check=True)
            commit(rows, folder / 'transaction', plan_hash, precommit, postcheck)
        print(json.dumps({'status': 'INSTALLED_LIVE_BROWSER_PENDING' if args.apply else 'PREPARED_NOT_APPLIED',
                          'output': str(folder), 'targets': len(plan['records'])}))


if __name__ == '__main__':
    main()
