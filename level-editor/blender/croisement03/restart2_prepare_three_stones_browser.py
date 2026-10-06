"""Prepare a guarded private browser fixture without copying whole-map payloads."""
import copy
import hashlib
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from prepare_publication_browser import bound_patches, preserved_ungrouped
from scene_manifest import scene_metadata
from stored_map import expand_document
from asset_index import write_asset_index, discover_asset_index
from canonical_assets import read_model


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def read(p):
    return json.loads(p.read_text())


def write(p, data):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(data, indent=2)+'\n')


def main():
    stage = ROOT / 'level-editor/work/croisement03-refinement/restart2/approved-stone-integration-preflight/three-stones-stage-v1'
    live = ROOT / 'level-editor/library'
    receipt = read(stage / 'receipt.json')
    guards = copy.deepcopy(receipt['live_guards'])
    for p,h in guards.items(): assert sha(Path(p)) == h
    out = stage / 'browser-preparation-v1'; out.mkdir(exist_ok=False)
    library = out / 'library'; links = {}
    selected = set(receipt['assets']); retired = set(receipt['retired_native_assets'])
    stored = read(stage / 'croisement03.rhlos-map.json')
    previous = read(live / 'scenes/croisement03.rhlos-map.json')
    assert [p for p in stored['placements'] if not set(p['assets']) & selected] == [p for p in previous['placements'] if not set(p['assets']) & retired]
    references = {r['id']:r for r in stored['assetSources']+stored['sceneAssets']}
    def link(relative, source):
        target = library / relative
        if target.exists():
            assert sha(target) == sha(source); return
        target.parent.mkdir(parents=True, exist_ok=True)
        os.link(source,target)
        assert source.stat().st_ino == target.stat().st_ino
        links[relative] = dict(source=str(source),sha256=sha(source))
        if source.is_relative_to(live): guards[str(source)] = sha(source)
    for identity, ref in references.items():
        base = stage / 'map-assets' if identity in selected else live
        descriptor = read(base / ref['descriptor'])
        assert sha(base / ref['descriptor']) == ref['descriptor_sha256']
        assert sha(base / ref['model']) == ref['model_sha256']
        for relative in [ref['descriptor'],ref['model']]+[r['path'] for r in descriptor.get('resources',[])]:
            link(relative,base / relative)
    document = expand_document(library,stored)
    prior = expand_document(live,previous)
    ungrouped = preserved_ungrouped(document,prior)
    qualified = set()
    for ref in stored['assetSources']:
        desc = read(library / ref['descriptor']); model,_,_ = read_model(library / ref['model'],library)
        names = {n.get('name') for n in model['nodes']}
        for part in desc['parts']:
            assert part['node'] in names
            qualified.add('asset:'+ref['id']+':'+part['node'])
    assert {o['node'] for o in document['objects']} == qualified
    entries = [e for e in discover_asset_index(library / '3d-assets')['assets'] if e['id'] in references]
    assert {e['id'] for e in entries} == set(references)
    index_path = out / 'private-index.json'
    write_asset_index(library / '3d-assets',target=index_path,descriptors=[e['descriptor'] for e in entries])
    files = {}
    def add(relative,path):
        record = dict(path=relative,url='/@fs/'+str(path.resolve(strict=True)),sha256=sha(path))
        if relative in files: assert files[relative] == record
        files[relative] = record
    for relative in links: add(relative,library / relative)
    add('3d-assets/index.json',index_path)
    add('scenes/croisement03.rhlos-map.json',stage / 'croisement03.rhlos-map.json')
    game_index = live / 'game-data/index.json'; add('game-data/index.json',game_index)
    for relative in read(game_index)['files']: add('game-data/'+relative,live / 'game-data' / relative)
    model = scene_metadata(library,stored); generated = {}
    for mat in model['materials']:
        identity = mat.get('extras',{}).get('generated_source_sha256')
        if identity: generated[identity] = generated.get(identity,0)+1
    patches = bound_patches(model['nodes'],document)
    aliases = {p['assets'][0]:p['id'] for p in stored['placements'] if p['id'].startswith('group-')}
    assert all(k=='croisement03-'+v for k,v in aliases.items())
    for p,h in guards.items(): assert sha(Path(p)) == h
    config = dict(map='croisement03',mode='staged',files=list(files.values()),stage=str(stage),
        shared_module_url='/@fs/'+str(ROOT / 'level-editor/shared/src/index.ts'),
        protected_live_files=guards,audit_timeout_ms=900000,
        expected=dict(groups=len(document['groups']),parts=len(document['objects']),ungrouped_parts=ungrouped,
            width=stored['size'][0],assets=entries,base_asset_ids=sorted(references),new_asset_ids=sorted(selected),
            generated_materials=generated,required_patches=sorted(patches)))
    write(out / 'config.json',config)
    write(out / 'receipt.json',dict(status='PRIVATE_CONFIG_READY; browser launch awaits coordinated slot',
        config_sha256=sha(out / 'config.json'),groups=len(document['groups']),parts=len(document['objects']),
        referenced_assets=len(references),fixture_files=len(files),read_only_payload_hardlinks=links,
        protected_live_files=guards,legacy_aliases=aliases,untouched_placements_exact=True,
        limitations=['Hardlinked payloads are read-only fixture inputs; do not mutate them.',
                    'No browser launched, no live index or map changed; surrounding terrain/vegetation unfinished.']))
    print(out / 'config.json')


if __name__ == '__main__':
    main()
