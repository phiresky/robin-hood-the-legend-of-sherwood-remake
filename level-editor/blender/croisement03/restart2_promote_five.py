"""Hash-guarded scoped promotion after the five-asset browser audit passes."""
import argparse
import json
import shutil
import sys
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from promote_staged_publication import sha, library_lock, _apply, asset_file_pairs
import promote_staged_publication as promotion
from asset_index import write_asset_index

WORK = ROOT / 'level-editor/work/croisement03-refinement/restart2'
STAGE = WORK / 'publication-five-v1'
LIVE = ROOT / 'level-editor/library'


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def prepare():
    assert json.loads((STAGE / 'browser/result.json').read_text())['status'] == 'PASS'
    evidence = json.loads((STAGE / 'preparation.json').read_text())
    for path, expected in evidence['live_guards'].items():
        assert sha(Path(path)) == expected, path
    source = STAGE / 'map-assets/3d-assets'
    index = json.loads((source / 'index.json').read_text())
    selected = set(evidence['new_asset_ids'])
    live_index = LIVE / '3d-assets/index.json'
    prior = json.loads(live_index.read_text())
    old = next(a for a in prior['assets'] if a['id'] == 'croisement03-group-049')
    assert not selected.intersection(a['id'] for a in prior['assets'])
    pairs = []
    for entry in index['assets']:
        if entry['id'] not in selected:
            continue
        pairs += asset_file_pairs(source, LIVE / '3d-assets', entry)
        for key in ('lossy_model', 'preview_model'):
            assert entry.get(key), (entry['id'], key)
            for relative in (entry[key], entry[key] + '.receipt.json'):
                pairs.append((source / relative, LIVE / '3d-assets' / relative))
    old_folder = LIVE / '3d-assets' / Path(old['descriptor']).parent
    rollback = STAGE / 'retired-group049-complete-backup'
    shutil.copytree(old_folder, rollback)
    # Keep old geometry payloads on disk; retire just its discoverable descriptor.
    pairs.append((None, LIVE / '3d-assets' / old['descriptor']))
    pairs.sort(key=lambda pair: pair[1].name == 'asset.json')
    pairs.append((STAGE / 'croisement03.rhlos-map.json', LIVE / 'scenes/croisement03.rhlos-map.json'))
    prospective = {str(target.relative_to(LIVE / '3d-assets')):source_path for source_path,target in pairs
                   if target.is_relative_to(LIVE / '3d-assets')}
    merged = STAGE / 'promotion-library-index.json'
    write_asset_index(LIVE / '3d-assets', target=merged, files=prospective)
    new = json.loads(merged.read_text())
    other = lambda rows: {a['id']:a for a in rows if a['id'] not in selected | {old['id']}}
    assert other(prior['assets']) == other(new['assets']), 'Unrelated palette entry changed'
    pairs.append((merged, live_index))
    records = []
    for i,(source_path,target) in enumerate(pairs):
        records.append(dict(source=str(source_path) if source_path else None,target=str(target),
            source_sha256=sha(source_path) if source_path else None,previous_sha256=sha(target),
            backup=str(STAGE / 'promotion-backup' / f'{i:03d}-{target.name}')))
    manifest = dict(status='PREPARED_NOT_APPLIED',stage=str(STAGE),library=str(LIVE),files=records,
        protected_files=[dict(path=p,sha256=h) for p,h in evidence['live_guards'].items()
                         if p != str(LIVE / 'scenes/croisement03.rhlos-map.json') and p != str(LIVE / '3d-assets' / old['descriptor'])],
        index_generation=dict(target=str(live_index)),browser_check=dict(status='PASS',result=str(STAGE / 'browser/result.json')),
        scope=dict(replaced=old['id'],approved=sorted(selected),unrelated_entries_unchanged=len(other(prior['assets']))))
    write(STAGE / 'palette-before.json', prior)
    write(STAGE / 'promotion.json', manifest)
    print('Prepared guarded scoped promotion; live files untouched')


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--apply', action='store_true'); args = parser.parse_args()
    with library_lock(LIVE):
        if not args.apply:
            assert not (STAGE / 'promotion.json').exists()
            prepare()
        else:
            old_path = LIVE / '3d-assets/croisement03/croisement03-group-049/asset.json'
            replacement = STAGE / 'map-assets/3d-assets/croisement03/croisement03-southwest-firewood-stack/asset.json'
            original_check = promotion.check_gameplay_preserved

            def retirement_check(source, target):
                if source is not None or target.resolve() != old_path:
                    return original_check(source, target)
                # One explicitly mapped retirement: replacement retains all gameplay
                # and every world-space obstacle coordinate, with the full old payload backed up.
                old, new = json.loads(old_path.read_text()), json.loads(replacement.read_text())
                assert old['gameplay'] == new['gameplay']
                assert [p['node'] for p in old['parts']] == [p['node'] for p in new['parts']] == ['building-049']
                previous = json.loads((LIVE / 'scenes/croisement03.rhlos-map.json').read_text())
                pose = next(p['transform'] for p in previous['placements'] if p['assets'] == [old['id']])
                origin = new['source_origin_scene']
                next_pose = dict(dx=origin[0],dy=-origin[1]*math.sin(math.radians(35)),dz=origin[2]*math.cos(math.radians(35)))
                a,b = old['parts'][0]['obstacle_local_game'],new['parts'][0]['obstacle_local_game']
                assert {k:v for k,v in a.items() if k!='points'} == {k:v for k,v in b.items() if k!='points'}
                for pa,pb in zip(a['points'],b['points'],strict=True):
                    for key,offset in [('x','dx'),('y','dy'),('z_bottom','dz'),('z_top','dz')]:
                        assert abs(pa[key]+pose[offset]-pb[key]-next_pose[offset]) < 1e-9
                for path in old_path.parent.rglob('*'):
                    if path.is_file():
                        assert sha(path) == sha(STAGE/'retired-group049-complete-backup'/path.relative_to(old_path.parent))

            promotion.check_gameplay_preserved = retirement_check
            try:
                _apply(STAGE / 'promotion.json')
            finally:
                promotion.check_gameplay_preserved = original_check
            prior = json.loads((STAGE / 'palette-before.json').read_text())
            after = json.loads((LIVE / '3d-assets/index.json').read_text())
            selected = set(json.loads((STAGE / 'scope.json').read_text())['asset_ids'])
            other = lambda rows:{a['id']:a for a in rows if a['id'] not in selected | {'croisement03-group-049'}}
            assert other(prior['assets']) == other(after['assets'])
            assert not any(a['id']=='croisement03-group-049' for a in after['assets'])
            assert selected <= {a['id'] for a in after['assets']}
            write(STAGE / 'live-promotion-result.json', dict(status='PASS',published=sorted(selected),
                unrelated_entries_exact=len(other(prior['assets'])),map_sha256=sha(LIVE / 'scenes/croisement03.rhlos-map.json'),
                limitations=['Scoped five assets only. Full map refinement remains incomplete.']))


if __name__ == '__main__':
    main()
