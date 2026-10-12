"""Reconcile the fence-only publication before a fresh support clearance sweep."""
import copy
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qualify as q

transaction = q.ROOT / 'level-editor/work/active/croisement02/recovery-fence100-tip-publication-v1/transaction-v1/root-install-v1/transaction'
journal_path = transaction / 'journal.json'
journal = json.loads(journal_path.read_text())
qualification = json.loads((q.OUT / 'qualification.json').read_text())
entry = next(f for f in journal['files'] if f['target'].endswith('/scenes/croisement02.rhlos-map.json'))
old_path, current_path = Path(entry['backup']), Path(entry['target'])
assert q.pin(old_path)['sha256'] == entry['old_sha256'] == qualification['map']['sha256']
assert q.pin(current_path)['sha256'] == entry['new_sha256'] == 'afa4178a7bf7cc529b731a3faba6ba1dd8e4a7ea6008d2f043a6f4f31fa8e4bb'
old, current = json.loads(old_path.read_text()), json.loads(current_path.read_text())
expected = copy.deepcopy(old)
assert old['assetSources'][86]['id'] == 'croisement02-southwest-field-wattle-fence'
expected['assetSources'][86]['model_sha256'] = '6d1a1008767c12230c87eda7b5c776b8c84007e41ef71b03504ef3afc8bef89b'
assert expected == current, 'Unexpected canonical changes require new reconciliation'
placements = {p['id']:p for p in current['placements']}
sources = {s['id']:s for s in current['assetSources']}
checked = []
for candidate in qualification['rows'][2]['bounds_candidates']:
    assert placements[candidate['id']] == candidate['placement']
    assert sources[candidate['id']]['model'] == candidate['model']
    assert q.pin(q.LIB / candidate['model'])['sha256'] == candidate['model_sha256']
    checked.append(candidate['id'])
receipt = q.OUT / 'profile03-map-requalification-v2.json'
assert not receipt.exists()
receipt.write_text(json.dumps(dict(status='EXACT_FENCE_ONLY_MAP_CHANGE_RECONCILED',
    prior_map=q.pin(old_path), current_map=q.pin(current_path), journal=q.pin(journal_path),
    sole_change='/assetSources/86/model_sha256', unchanged_profile03_candidates=checked,
    recipe=q.pin(Path(__file__))), indent=2)+'\n')
original = HERE / 'sweep_profile03_proposed_supports_v1.py'
source = original.read_text().replace('profile03-proposed-support-sweep-v1', 'profile03-proposed-support-sweep-v2').replace('profile03-proposed-supports-v1.json', 'profile03-proposed-supports-v2.json')
needle = "    namespace = dict(__name__='proposed_sweep'"
assert source.count(needle) == 1
source = source.replace(needle, "    source = source.replace(\"expected_map_sha256=qualification['map']['sha256']\", \"expected_map_sha256='" + entry['new_sha256'] + "'\")\n" + needle)
exec(compile(source, str(original), 'exec'), {'__name__':'__main__', '__file__':str(original)})
