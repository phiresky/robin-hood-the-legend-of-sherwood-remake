"""Stage the three approved trees privately, retaining unresolved foliage membership."""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from asset_index import write_asset_index
from unify_map_assets import stage
from hybrid_library import stage_hybrid

RUN = ROOT / 'level-editor/work/croisement03-refinement/restart2'
BASE = RUN / 'trio-tree-integration-v1'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def read(p):
    return json.loads(p.read_text())


def write(p, value):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(value, indent=2) + '\n')


def main():
    assert shutil.disk_usage(ROOT).free > 25 * 1024**3
    out = BASE / 'stage-v1'
    out.mkdir(exist_ok=False)
    parity = BASE / 'metadata-parity-v2'
    live = ROOT / 'level-editor/library'
    map_path = live / 'scenes/croisement03.rhlos-map.json'
    receipt = read(parity / 'receipt.json')
    assert sha(map_path) == receipt['live_map_sha256']
    document = read(map_path)
    before = copy.deepcopy(document)
    specs = [(f'croisement03-tree-{n}', RUN / f'tree{n}-exact-export-v2') for n in (12, 13, 14)]
    guards = {str(map_path): sha(map_path)}
    reports = {}
    retired = set()
    for identity, export in specs:
        report = read(parity / identity / 'parity.json')
        reports[identity] = report
        guards.update(report['protectedFiles'])
        retired.update(f['prior_asset'] for f in report['fragments'])
        exported = read(export / 'report.json')
        assert sha(export / 'model.glb') == exported['model_sha256']
        dest = out / 'input/3d-assets' / identity
        dest.mkdir(parents=True)
        shutil.copy2(export / 'model.glb', dest / 'model.glb')
        write(dest / 'asset.json', read(parity / identity / 'asset-metadata-proposal.json'))
    (out / 'input/scenes').mkdir()
    write_asset_index(out / 'input/3d-assets')
    stage(out / 'input', out / 'canonical/library')
    stage_hybrid(out / 'canonical/library', out / 'map-assets')
    index = read(out / 'map-assets/3d-assets/index.json')
    assert {a['id'] for a in index['assets']} == {s[0] for s in specs}
    document['assetSources'] = [r for r in document['assetSources'] if r['id'] not in retired]
    document['placements'] = [p for p in document['placements'] if not (set(p['assets']) & retired)]
    for entry in index['assets']:
        identity = entry['id']
        path = out / 'map-assets/3d-assets' / entry['descriptor']
        descriptor = read(path)
        proposal = read(parity / identity / 'asset-metadata-proposal.json')
        assert descriptor['gameplay'] == proposal['gameplay']
        assert descriptor['parts'] == proposal['parts']
        document['assetSources'].append(dict(id=identity, descriptor='3d-assets/' + entry['descriptor'],
            descriptor_sha256=sha(path), model='3d-assets/' + entry['model'],
            model_sha256=sha(out / 'map-assets/3d-assets' / entry['model']), model_scene=descriptor['model_scene']))
        document['placements'].append(dict(id=identity, assets=[identity], transform=reports[identity]['proposed_transform']))
    identities = {s[0] for s in specs}
    assert [p for p in document['placements'] if not set(p['assets']) & identities] == [p for p in before['placements'] if not set(p['assets']) & retired]
    for p, digest in guards.items():
        assert sha(Path(p)) == digest
    write(out / 'croisement03.rhlos-map.json', document)
    write(out / 'receipt.json', dict(status='PRIVATE_STAGED; browser and coordinated live publication pending',
        assets=sorted(identities), retired_native_assets=sorted(retired), live_guards=guards,
        world_parity_receipt=str(parity / 'receipt.json'), world_parity_receipt_sha256=sha(parity / 'receipt.json'),
        untouched_placements_preserved=True, canonical_gameplay_and_parts_exact=True,
        standalone_resources_only=True, live_library_changed=False,
        limitations=['Private map proposal retains existing references for untouched assets; no whole-map payload copy.',
                    'Static tree geometry and appearance approved; shared Arbre06 physical fragment membership remains provisional. Native animation is unchanged, and duplication/occlusion must be resolved before live publication.','No full terrain or wind completion claim.']))
    print(out)


if __name__ == '__main__':
    main()
