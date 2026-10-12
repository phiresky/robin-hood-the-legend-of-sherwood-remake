"""Freeze a real editor UI proof without launching a browser or publishing assets."""
import argparse, hashlib, json, shutil
from pathlib import Path
R=Path.cwd();LIB=R/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def prepare(out,refined=False,receiver_assets=None,crops=False,decode_workers=None,source_from=None,actor_plan=None):
 old=R/'level-editor/work/active/croisement02/approved-round-wood-integration-oct11-v1/canonical-postinstall-smoke-v2'
 reqpath=R/'level-editor/work/active/croisement02/restart217-butterfly05-private-contract-v1/request.json';req=json.loads(reqpath.read_text())
 for row in [req['contract'],req['model'],*req['source_files']]:assert sha(Path(row['path']))==row['sha256'],row
 out.mkdir(exist_ok=False)
 catalog=json.loads((LIB/'mission-states/index.json').read_text());assert not any(e['id']==req['entry']['id'] for e in catalog['entries']);catalog['entries'].append(req['entry']);dump(out/'candidate-catalog.json',catalog)
 routes={'/library/mission-states/index.json':str(out/'candidate-catalog.json')}
 if receiver_assets is not None:
  canonical=json.loads((LIB/'scenes/croisement02.rhlos-map.json').read_text());selected=set(receiver_assets)
  known={row['id'] for row in canonical['assetSources']};assert selected<=known,selected-known
  minimal={**canonical,'assetSources':[row for row in canonical['assetSources'] if row['id'] in selected],'placements':[row for row in canonical['placements'] if set(row['assets'])<=selected]}
  assert {a for row in minimal['placements'] for a in row['assets']}==selected
  dump(out/'bounded-receiver-map.json',minimal)
  routes['/library/scenes/croisement02.rhlos-map.json']=str(out/'bounded-receiver-map.json')
  dump(out/'receiver-scope.json',{'scope':'Bounded actual UI proof only; full canonical map memory and composition verification remains required','canonical_map_sha256':sha(LIB/'scenes/croisement02.rhlos-map.json'),'canonical_asset_count':len(canonical['assetSources']),'included_asset_ids':sorted(selected),'scene_assets':canonical['sceneAssets'],'source_mission_and_level_unchanged':True})

 for p in Path(req['library_root']).rglob('*'):
  if p.is_file():routes['/library/'+str(p.relative_to(req['library_root']))]=str(p)
 authority=json.loads((old/'runtime-authority.json').read_text());snap={}
 for directory in authority['sourceDirectories']:
  for p in sorted((R/directory).rglob('*')):
   if not p.is_file():continue
   key=str(p.relative_to(R));snap[key]=sha(p)
   if p.suffix in {'.ts','.tsx','.js','.jsx','.mjs','.cjs','.css','.json'}:
    dst=out/'snapshot'/key;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dst)
 if source_from is not None:
  baseline=json.loads((source_from/'source-snapshot.json').read_text())
  assert set(snap)==set(baseline),'Source membership changed between diagnostic packets'
  for key,expected in baseline.items():
   src=source_from/'snapshot'/key
   if src.exists():
    dst=out/'snapshot'/key;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,dst)
   else:assert sha(R/key)==expected
  snap=baseline
 if decode_workers is not None:
  assert receiver_assets is None and decode_workers in (1,8)
  key='level-editor/app/src/map-candidate.ts';path=out/'snapshot'/key
  source=path.read_text();oldline='await Promise.all(Array.from({ length: Math.min(8, references.length) }, worker));'
  assert source.count(oldline)==1
  original=snap[key];path.write_text(source.replace(oldline,oldline.replace('Math.min(8,','Math.min('+str(decode_workers)+',')))
  snap[key]=sha(path)
  dump(out/'source-variant.json',{'scope':'Frozen diagnostic only; canonical source unchanged','path':key,'original_sha256':original,'served_sha256':snap[key],'asset_decode_workers':decode_workers,'texture_encode_workers':2,'source_statement':oldline,'served_statement':oldline.replace('Math.min(8,','Math.min('+str(decode_workers)+',')})
 authority['membership']=sorted(snap);authority['index']={'path':str(LIB/'3d-assets/index.json'),'initial_sha256':sha(LIB/'3d-assets/index.json')}
 dump(out/'source-snapshot.json',snap);dump(out/'runtime-authority.json',authority)
 pins={str(p):sha(p) for p in [reqpath,LIB/'3d-assets/index.json',LIB/'scenes/croisement02.rhlos-map.json',LIB/'mission-states/index.json',*[Path(p)for p in routes.values()]]}
 if decode_workers is not None:
  data_pins={};canonical=json.loads((LIB/'scenes/croisement02.rhlos-map.json').read_text())
  for reference in [*canonical['sceneAssets'],*canonical['assetSources']]:
   for field in ['model','descriptor']:data_pins[str(LIB/reference[field])]=reference[field+'_sha256']
   descriptor=json.loads((LIB/reference['descriptor']).read_text())
   for resource in descriptor.get('resources',[]):data_pins[str(LIB/resource['path'])]=resource['sha256']
  dump(out/'canonical-resource-pins.json',data_pins);pins.update(data_pins)
 dump(out/'runtime-inputs.json',{**snap,**pins})
 for name in ['editor.tsx','observe-library.js']:shutil.copy2(old/name,out/name)
 shutil.copy2(R/'level-editor/blender/croisement02/native_physical_effect_ui_proof.js',out/'check.js')
 contract=json.loads(Path(req['contract']['path']).read_text());dump(out/'check-config.json',{'entry':req['entry'],'focus':contract['focus_element_id'],'refined':refined,'boundedReceiver':receiver_assets is not None,'receiverAssets':receiver_assets,'cropRegion':{'x':888,'y':900,'width':96,'height':108} if crops else None,'sourceOrigin':contract['native']['origin'],'expectedAssetIds':[r['id'] for r in canonical['assetSources']] if decode_workers is not None else None,'expectedSceneAssetIds':[r['id'] for r in canonical['sceneAssets']] if decode_workers is not None else None})
 if actor_plan is not None:
  plan=json.loads(actor_plan.read_text());pins[str(actor_plan)]=sha(actor_plan)
  for file,expected in plan['pins'].items():
   path=R/file;assert sha(path)==expected,file;pins[str(path)]=expected
  oracle_root=out/'oracle';oracle_root.mkdir()
  for name in ['actor_crossing_oracle.mjs','hidden-character-outline/outline.mjs','hidden-character-outline/mask-query.mjs','native-character-masks/character-masks.mjs']:
   src=R/'level-editor/blender/croisement02'/name;dst=oracle_root/name;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,dst)
   if name=='actor_crossing_oracle.mjs':dst.write_text(dst.read_text().replace('../../app/src/native-state-presentation.ts','/@fs'+str(R/'level-editor/app/src/native-state-presentation.ts')))
   pins[str(dst)]=sha(dst)
  shutil.copy2(R/'level-editor/blender/croisement02/actor_crossing_ui_proof.mjs',out/'actor-proof.mjs')
  config=json.loads((out/'check-config.json').read_text());config['actorCrossing']={**plan,'oracleModule':'/@fs'+str(oracle_root/'actor_crossing_oracle.mjs'),'proofModule':'/@fs'+str(out/'actor-proof.mjs')};dump(out/'check-config.json',config)
  dump(out/'runtime-inputs.json',{**snap,**pins})
 run=(old/'run.mjs').read_text().replace(str(old.relative_to(R)),str(out.relative_to(R)))
 run=run.replace('const privateRoutes=new Map();','const privateRoutes=new Map(Object.entries('+json.dumps(routes)+'));')
 run=run.replace("if(p.stage.name!=='canonical-source-camera')throw Error('Unexpected capture');", "if(!/^b05-(physical-over-art|original-art|refined-map|comparison-crop|actor-case[0-9]+)-tick[0-9]+$/.test(p.stage.name))throw Error('Unexpected capture');")
 run=run.replace("await evaluate('window.scatterCaptures[\"canonical-source-camera\"]=true')", "await evaluate('window.scatterCaptures['+JSON.stringify(p.stage.name)+']=true')")
 run=run.replace("await readFile(join(privateLibrary,path))", "await readFile(privateRoutes.get('/library/'+path)??join(privateLibrary,path))")
 run=run.replace("if(s?.startsWith('FAIL'))throw Error(s);", "await writeFile(join(out,'readiness.json'),JSON.stringify({at:Date.now(),status:s}));if(s?.startsWith('FAIL'))throw Error(s);")
 run=run.replace("await writeFile(join(out,'canonical-checkpoint.json'),JSON.stringify(result,null,2));", "await writeFile(join(out,'canonical-checkpoint.json'),JSON.stringify(result,null,2));const crops=await evaluate('window.nativeEffectCrops??[]');for(const row of crops){if(!/^b05-(crop|actor-case[0-9]+)-(original-art|physical-over-art|refined-map)-tick(0|74)$/.test(row.name))throw Error('Unexpected crop');await writeFile(join(out,row.name+'.png'),Buffer.from(row.png.split(',')[1],'base64'));delete row.png;}await writeFile(join(out,'source-pixel-crops.json'),JSON.stringify(crops,null,2));")
 if decode_workers is not None:
  run=run.replace('let ready=false;for',"let lastReadyStatus;let ready=false;for")
  run=run.replace("await writeFile(join(out,'readiness.json'),JSON.stringify({at:Date.now(),status:s}));", "await writeFile(join(out,'readiness.json'),JSON.stringify({at:Date.now(),status:s}));if(s!==lastReadyStatus){lastReadyStatus=s;await appendFile(join(out,'readiness-history.jsonl'),JSON.stringify({at:Date.now(),status:s,resources:resourceSample()})+'\\n');}")
  run=run.replace("if(!ready)throw Error('Editor readiness timeout');", "if(!ready)throw Error('Editor readiness timeout');hostEvent('full-canonical-map-ready');")
 (out/'run.mjs').write_text(run)
 for p in out.iterdir():
  if p.is_file():pins[str(p)]=sha(p)
 dump(out/'inputs.json',pins)
 dump(out/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(out/'run.mjs'),'run_sha256':sha(out/'run.mjs'),'scope':('Bounded receiver subset; full canonical scene proof remains required. ' if receiver_assets is not None else '')+'Actual editor UI physical-effects/native-art prototype, normal frame seek and loop restoration. Original camera. No geometry approval or whole-map completion.','input_request':str(reqpath),'input_sha256':sha(reqpath)})
 print(out)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);p.add_argument('--refined',action='store_true');p.add_argument('--receiver-assets',nargs='*',default=None);p.add_argument('--crops',action='store_true');p.add_argument('--decode-workers',type=int,choices=[1,8]);p.add_argument('--source-from',type=Path);p.add_argument('--actor-plan',type=Path);a=p.parse_args();prepare(a.out.resolve(),a.refined,a.receiver_assets,a.crops,a.decode_workers,a.source_from.resolve() if a.source_from else None,a.actor_plan.resolve() if a.actor_plan else None)
