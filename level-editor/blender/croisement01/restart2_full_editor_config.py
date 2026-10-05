"""Bind legacy instance-based Crois01 publication to the unchanged browser verifier."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from stored_map import expand_document
from scene_manifest import scene_metadata
from prepare_publication_browser import bound_patches
from asset_index import write_asset_index
from review_evidence import sha
library=ROOT/'level-editor/library';stage=ROOT/'level-editor/work/croisement01-refinement/restart2/tree18-integration-v1'
revision = sys.argv[1] if len(sys.argv) > 1 else 'full-editor-live-v2'
assert revision.startswith('full-editor-live-v') and Path(revision).name == revision
out=stage/revision;out.mkdir(exist_ok=False)
scene=library/'scenes/croisement01.rhlos-map.json';saved=json.loads(scene.read_text());doc=expand_document(library,saved)
assert len(saved['placements'])==59 and len(doc['groups'])==58 and len(doc['objects'])==91
proof=json.loads((stage/'scene-splice-proof.json').read_text());assert sha(scene)==proof['staged_scene_sha256']
index=json.loads((library/'3d-assets/index.json').read_text());ids={r['id'] for r in doc['assetSources']+doc['sceneAssets'] if r.get('descriptor')};entries=[e for e in index['assets'] if e['id'] in ids];assert len(entries)==len(ids)
private=out/'private-index.json';write_asset_index(library/'3d-assets',target=private,descriptors=[e['descriptor'] for e in entries]);entries=json.loads(private.read_text())['assets']
files={}
def add(name,path):
 path=Path(path).resolve(strict=True);record=dict(path=name,url='/@fs/'+str(path),sha256=sha(path));assert name not in files or files[name]==record;files[name]=record
add('scenes/croisement01.rhlos-map.json',scene);add('3d-assets/index.json',private)
for ref in doc['sceneAssets']+doc['assetSources']:
 add(ref['model'],library/ref['model'])
for e in entries:
 for key in ['descriptor','model','lossy_model','preview_model']:
  if e.get(key):add('3d-assets/'+e[key],library/'3d-assets'/e[key])
 if e.get('lossy_model'):add('3d-assets/'+e['lossy_model']+'.receipt.json',library/'3d-assets'/(e['lossy_model']+'.receipt.json'))
 d=json.loads((library/'3d-assets'/e['descriptor']).read_text())
 for resource in d.get('resources',[]):add(resource['path'],library/resource['path'])
game=library/'game-data/index.json'
if game.exists():
 add('game-data/index.json',game)
 for name in json.loads(game.read_text())['files']:add('game-data/'+name,library/'game-data'/name)
metadata=scene_metadata(library,doc);generated={}
for material in metadata.get('materials',[]):
 identity=material.get('extras',{}).get('generated_source_sha256')
 if identity:generated[identity]=generated.get(identity,0)+1
protected={item['url'].removeprefix('/@fs/'):item['sha256'] for item in files.values()}
config=dict(map='croisement01',mode='live',files=list(files.values()),shared_module_url='/@fs/'+str(ROOT/'level-editor/shared/src/index.ts'),expected=dict(groups=len(doc['groups']),parts=len(doc['objects']),ungrouped_parts=sum(not o.get('group') for o in doc['objects']),width=doc['size'][0],assets=entries,base_asset_ids=sorted(ids),new_asset_ids=['croisement01-tree-18'],generated_materials=generated,required_patches=sorted(bound_patches(metadata['nodes'],doc))),stage=str(stage),protected_live_files=protected,audit_timeout_ms=600000,preparation_note='Legacy map placement IDs intentionally differ from asset-local group IDs. Runtime hydration, unchanged58 other placements and exactpart ownership are audited; full standard editor selection/insertion verifier is unchanged.')
(out/'checker-provenance.json').write_text(json.dumps({str(p.relative_to(ROOT)):sha(p) for p in [ROOT/'level-editor/refinement/browser/verify_publication.mjs',ROOT/'level-editor/refinement/browser/publication-check.js']},indent=2)+'\n')
(out/'config.json').write_text(json.dumps(config,indent=2)+'\n');print(json.dumps(dict(config=str(out/'config.json'),files=len(files),groups=len(doc['groups']),parts=len(doc['objects']))))
