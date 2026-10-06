"""Prepare one exact hall endpoint for full-map and insertion/reload verification."""
import copy,hashlib,json,subprocess,sys
from pathlib import Path
STATE=sys.argv[1]
assert STATE in ('initial-initial','initial-applied','applied-initial','applied-applied','independent-wiring-v2')
ROOT=Path(__file__).resolve().parents[3];LIB=ROOT/'level-editor/library';BASE=ROOT/'level-editor/work/york-refinement/restart2/hall-textures-v2/exports-v3'/STATE;OUT=BASE/'editor-stage-v1'
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
original=json.loads((LIB/'scenes/york.rhlos-map.json').read_text());index=json.loads((BASE/'3d-assets/index.json').read_text());ids={r['id'] for r in index['assets']};document=copy.deepcopy(original);pins={str(LIB/'scenes/york.rhlos-map.json'):sha(LIB/'scenes/york.rhlos-map.json')};mapping={}
for reference in document['assetSources']+document['sceneAssets']:
 selected=reference['id'] in ids
 for field in ['model','descriptor']:
  live=LIB/reference[field];pins[str(live)]=sha(live);file=BASE/'3d-assets'/reference['id']/('model.glb' if field=='model' else 'asset.json') if selected else live
  mapping[reference[field]]=file;reference[field+'_sha256']=sha(file)
 if selected:
  reference.pop('model_scene',None)
 descriptor=json.loads(mapping[reference['descriptor']].read_text())
 for resource in descriptor.get('resources',[]):
  file=LIB/resource['path'];assert sha(file)==resource['sha256'];mapping[resource['path']]=file;pins[str(file)]=sha(file)
assert document['placements']==original['placements']
if STATE=='independent-wiring-v2':
 placement=next(p for p in document['placements'] if p['id']=='york-castle-great-hall')
 placement['patches']={'york-castle-great-hall':{'appearance-1':'patch-001','appearance-2':'patch-002'}}
for entry in index['assets']:
 for field in ['model','descriptor','lossy_model','preview_model']:
  if field not in entry:continue
  value=entry[field];mapping['3d-assets/york/'+value]=BASE/'3d-assets'/value;entry[field]='york/'+value
live_index=json.loads((LIB/'3d-assets/index.json').read_text());all_ids={r['id'] for r in original['assetSources']+original['sceneAssets']};replacements={r['id']:r for r in index['assets']};full_index={'version':1,'assets':[copy.deepcopy(replacements.get(r['id'],r)) for r in live_index['assets'] if r['id'] in all_ids]}
for entry in full_index['assets']:
 if entry['id'] in ids:entry.pop('lossy_model',None)
 for field in ['model','descriptor','lossy_model','preview_model']:
  if field in entry and '3d-assets/'+entry[field] not in mapping:mapping['3d-assets/'+entry[field]]=LIB/'3d-assets'/entry[field]
for file in mapping.values():
 if file.is_relative_to(LIB):pins[str(file)]=sha(file)
OUT.mkdir()
for scope,visual in [('full-map',True),('hall-functional',False)]:
 folder=OUT/scope;folder.mkdir();doc=copy.deepcopy(document);palette=copy.deepcopy(full_index)
 if not visual:palette['assets']=[r for r in palette['assets'] if r['id'] in ids or r.get('editor_usage')=='map-background']
 palette_path=folder/'palette-index.json';palette_path.write_text(json.dumps(palette,indent=2)+'\n')
 if not visual:
  doc['placements']=[p for p in doc['placements'] if p['id'] in ids];doc['assetSources']=[r for r in doc['assetSources'] if r['id'] in ids]
 scene=folder/'york.rhlos-map.json';scene.write_text(json.dumps(doc,indent=2)+'\n');files=dict(mapping);files['scenes/york.rhlos-map.json']=scene;files['3d-assets/index.json']=palette_path
 if not visual:
  allowed={'3d-assets/index.json','scenes/york.rhlos-map.json'}
  for r in doc['assetSources']+doc['sceneAssets']:
   allowed.update([r['model'],r['descriptor']]);allowed.update(x['path'] for x in json.loads(files[r['descriptor']].read_text()).get('resources',[]))
  for entry in palette['assets']:
   for field in ['model','descriptor','lossy_model','preview_model']:
    if field in entry:allowed.add('3d-assets/'+entry[field])
  files={k:v for k,v in files.items() if k in allowed}
 for file in sorted((LIB/'game-data').rglob('*')):
  if file.is_file():files[file.relative_to(LIB).as_posix()]=file
 config={'map':'york','mode':'staged','visual_only':visual,'shared_module_url':'/@fs/'+str(ROOT/'level-editor/shared/src/index.ts'),'files':[{'path':path,'url':'/@fs/'+str(file),'sha256':sha(file)} for path,file in sorted(files.items())],'protected_live_files':pins,'browser_profile_root':'/home/phire/.cache/york-hall-editor-browser','cdp_timeout_ms':300000,'audit_timeout_ms':600000,'reload_timeout_ms':120000,'expected':{'groups':len(doc['placements']),'parts':sum(len(json.loads(files[r['descriptor']].read_text())['parts']) for r in doc['assetSources']),'width':doc['size'][0],'assets':palette['assets'],'required_patches':['patch-001','patch-002'] if STATE=='independent-wiring-v2' else []}}
 (folder/'config.json').write_text(json.dumps(config,indent=2)+'\n')
 subprocess.run(['node',str(ROOT/'level-editor/blender/york/restart2_pair_editor_counts.mjs'),str(folder/'config.json')],check=True)
(OUT/'stage-guards.json').write_text(json.dumps({'status':'Private pinned editor inputs','live_map_sha256':pins[str(LIB/'scenes/york.rhlos-map.json')],'placement_transforms_unchanged':True,'appearance_bindings_added':STATE=='independent-wiring-v2','state':STATE,'selected_ids':sorted(ids),'modified_references_only':sorted(ids),'model_choice':'Exact full approved hall endpoint GLB; unrelated assets retain normal live catalog derivatives','protected_live_files':pins},indent=2)+'\n')
print(OUT)
