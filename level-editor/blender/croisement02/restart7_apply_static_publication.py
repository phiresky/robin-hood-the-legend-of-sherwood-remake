"""Apply a guarded static publication after verified gameplay migration.

Superseded descriptors may be retired only when their exact saved bytes, complete
migration authority, compiled semantics and actual Editor proof all match.
The shared publisher retains backups, checks every source/target and writes the
index last. Visual payloads and historical model resources are never retired.
"""
import argparse
import json
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
import promote_staged_publication as publisher

def validate(path):
    manifest = json.loads(path.read_text())
    if manifest['status'] != 'PREPARED_NOT_APPLIED':
        raise ValueError('Expected an unapplied frozen transaction')
    authority = manifest['retirement_authority']
    handoff_path = Path(authority['handoff'])
    if publisher.sha(handoff_path) != authority['handoff_sha256']:
        raise ValueError('Migration handoff changed')
    handoff = json.loads(handoff_path.read_text())
    semantic = Path(authority['semantic_proof'])
    if publisher.sha(semantic) != authority['semantic_sha256'] or not json.loads(semantic.read_text())['pass']:
        raise ValueError('Exact runtime semantic proof required')
    proof = manifest['final_editor_proof']
    if not proof.get('files'):
        raise ValueError('Missing bound Editor evidence files')
    for filename, expected in proof['files'].items():
        if publisher.sha(Path(filename)) != expected:
            raise ValueError('Final Editor proof changed: ' + filename)
    result_path = Path(proof['result'])
    if str(result_path) not in proof['files']:
        raise ValueError('Editor result must be hash-bound')
    result = json.loads(result_path.read_text())
    if result.get('status') != 'PASS' or not result.get('inputs'):
        raise ValueError('Actual Editor result is not a complete PASS')
    stage = Path(manifest['stage']).resolve()
    map_path = stage / 'croisement02.rhlos-map.json'
    if result.get('map_sha256') != publisher.sha(map_path):
        raise ValueError('Editor proof belongs to another map')
    if any(publisher.sha(Path(name)) != expected for name, expected in result['inputs'].items()):
        raise ValueError('A final reviewed input changed')
    source_inputs = {str(Path(row['source']).resolve()): row['source_sha256']
                     for row in manifest['files'] if row['source'] is not None
                     and row['target'] != manifest['index_generation']['target']}
    if any(result['inputs'].get(name) != expected or publisher.sha(Path(name)) != expected
           for name, expected in source_inputs.items()):
        raise ValueError('Editor proof omits or mismatches transaction inputs')
    semantic_binding = json.loads(Path(proof['semantic_binding']).read_text())
    if str(Path(proof['semantic_binding'])) not in proof['files'] or semantic_binding.get('status') != 'PASS' or semantic_binding.get('map_sha256') != result['map_sha256'] or semantic_binding.get('inputs') != result['inputs']:
        raise ValueError('Semantic proof is not bound to exact Editor inputs')
    if semantic_binding.get('semantic_sha256') != authority['semantic_sha256']:
        raise ValueError('Semantic result differs from migration authority')
    if proof['status'] != 'PASS':
        raise ValueError('Final Editor proof has not passed')
    retirements = {Path(row['path']).resolve(): row for row in authority['descriptors']}
    if {row['id'] for row in retirements.values()} != set(handoff['retire_ids']):
        raise ValueError('Retirement set differs from complete migration')
    deletions = {Path(row['target']).resolve() for row in manifest['files'] if row['source'] is None}
    if deletions != set(retirements):
        raise ValueError('Every deletion must exactly match the full retirement set')
    document = json.loads((Path(manifest['stage']) / 'croisement02.rhlos-map.json').read_text())
    placed = {asset for placement in document['placements'] for asset in placement['assets']}
    if placed.intersection(handoff['retire_ids']):
        raise ValueError('A retired asset remains placed')
    for target, row in retirements.items():
        if target.name != 'asset.json' or publisher.sha(target) != row['sha256']:
            raise ValueError('Retired descriptor changed: ' + str(target))
        if json.loads(target.read_text()).get('id') != row['id']:
            raise ValueError('Retirement descriptor identity mismatch')
    return manifest, retirements

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    manifest, retirements = validate(args.manifest)
    if not args.apply:
        print('PASS guarded publication preflight', len(manifest['files']), 'files')
        return
    original = publisher.check_gameplay_preserved
    def checked_preservation(source, target):
        row = retirements.get(target.resolve())
        if source is None and row is not None:
            if publisher.sha(target) != row['sha256']:
                raise ValueError('Retirement source changed during apply')
            return
        original(source, target)
    publisher.check_gameplay_preserved = checked_preservation
    publisher.apply(args.manifest)

if __name__ == '__main__':
    main()
