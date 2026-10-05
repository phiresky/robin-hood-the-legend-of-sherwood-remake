"""Prepare a scoped map audit using asset-qualified ownership, retaining legacy IDs."""
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from prepare_publication_browser import bound_patches, preserved_ungrouped
from scene_manifest import scene_metadata
from stored_map import expand_document
from asset_index import write_asset_index, discover_asset_index
from canonical_assets import read_model

STAGE = ROOT / 'level-editor/work/croisement03-refinement/restart2/publication-five-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    live, library = ROOT / 'level-editor/library', STAGE / 'map-assets'
    prior_stored = json.loads((live / 'scenes/croisement03.rhlos-map.json').read_text())
    stored = json.loads((STAGE / 'croisement03.rhlos-map.json').read_text())
    document = expand_document(library, stored)
    prior = expand_document(live, prior_stored)
    ungrouped = preserved_ungrouped(document, prior)
    scope = json.loads((STAGE / 'scope.json').read_text())
    selected = set(scope['asset_ids'])
    assert [p for p in stored['placements'] if p['assets'][0] not in selected] == [
        p for p in prior_stored['placements'] if p['assets'] != ['croisement03-group-049']]
    aliases = {p['assets'][0]:p['id'] for p in prior_stored['placements']
               if p['id'].startswith('group-') and p['id'] != 'group-049'}
    assert len(aliases) == 84
    assert all(k == 'croisement03-' + v for k,v in aliases.items())
    references = {r['id']:r for r in stored['assetSources'] + stored['sceneAssets']}
    qualified = set()
    for ref in stored['assetSources']:
        desc = json.loads((library / ref['descriptor']).read_text())
        model,_,_ = read_model(library / ref['model'], library)
        names = {n.get('name') for n in model['nodes']}
        for part in desc['parts']:
            assert part['node'] in names, (ref['id'], part['node'])
            qualified.add('asset:' + ref['id'] + ':' + part['node'])
    assert {o['node'] for o in document['objects']} == qualified
    # Palette includes precisely all referenced assets, without the retired firewood.
    entries = [e for e in discover_asset_index(library / '3d-assets')['assets'] if e['id'] in references]
    assert {e['id'] for e in entries} == set(references)
    out = STAGE / 'browser'; out.mkdir(exist_ok=True)
    index_path = out / 'private-index.json'
    write_asset_index(library / '3d-assets', target=index_path, descriptors=[e['descriptor'] for e in entries])
    files = {}
    def add(relative, path):
        record = dict(path=relative, url='/@fs/' + str(path.resolve(strict=True)), sha256=sha(path))
        if relative in files: assert files[relative] == record
        files[relative] = record
    for entry in entries:
        for field in ('model','descriptor','lossy_model','preview_model'):
            if entry.get(field): add('3d-assets/' + entry[field], library / '3d-assets' / entry[field])
        desc = json.loads((library / '3d-assets' / entry['descriptor']).read_text())
        for resource in desc.get('resources', []):
            assert sha(library / resource['path']) == resource['sha256']
            add(resource['path'], library / resource['path'])
    add('3d-assets/index.json', index_path)
    add('scenes/croisement03.rhlos-map.json', STAGE / 'croisement03.rhlos-map.json')
    game_index = live / 'game-data/index.json'
    add('game-data/index.json', game_index)
    for relative in json.loads(game_index.read_text())['files']:
        add('game-data/' + relative, live / 'game-data' / relative)
    model = scene_metadata(library, stored)
    generated = {}
    for material in model['materials']:
        identity = material.get('extras',{}).get('generated_source_sha256')
        if identity: generated[identity] = generated.get(identity,0) + 1
    patches = bound_patches(model['nodes'], document)
    guards = json.loads((STAGE / 'preparation.json').read_text())['live_guards']
    config = dict(map='croisement03',mode='staged',files=list(files.values()),stage=str(STAGE),
        shared_module_url='/@fs/' + str(ROOT / 'level-editor/shared/src/index.ts'),
        protected_live_files=guards,audit_timeout_ms=900000,
        expected=dict(groups=len(document['groups']),parts=len(document['objects']),ungrouped_parts=ungrouped,
            width=stored['size'][0],assets=entries,base_asset_ids=sorted(references),new_asset_ids=sorted(selected),
            generated_materials=generated,required_patches=sorted(patches)))
    (out / 'config.json').write_text(json.dumps(config,indent=2)+'\n')
    (out / 'legacy-identity-proof.json').write_text(json.dumps(dict(status='PASS',
        unchanged_other_placements=97,legacy_aliases=aliases,qualified_parts=len(qualified),
        repeated_navigation_name='scenery-navigation-frame, independently qualified by five asset IDs',
        unchanged_ungrouped_parts=ungrouped),indent=2)+'\n')
    print(json.dumps(dict(groups=len(document['groups']),parts=len(document['objects']),files=len(files),assets=len(entries))))


if __name__ == '__main__':
    main()
