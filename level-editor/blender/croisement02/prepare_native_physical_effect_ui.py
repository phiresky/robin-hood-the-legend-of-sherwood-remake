"""Freeze a real editor UI proof without launching a browser or publishing assets."""
import argparse, hashlib, json, shutil
from pathlib import Path
R=Path.cwd();LIB=R/'level-editor/library'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def prepare(out,refined=False,receiver_assets=None,crops=False):
 old=R/'level-editor/work/croisement02-refinement/approved-round-wood-integration-oct11-v1/canonical-postinstall-smoke-v2'
 reqpath=R/'level-editor/work/croisement02-refinement/restart217-butterfly05-private-contract-v1/request.json';req=json.loads(reqpath.read_text())
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
 authority['membership']=sorted(snap);authority['index']={'path':str(LIB/'3d-assets/index.json'),'initial_sha256':sha(LIB/'3d-assets/index.json')}
 dump(out/'source-snapshot.json',snap);dump(out/'runtime-authority.json',authority)
 pins={str(p):sha(p) for p in [reqpath,LIB/'3d-assets/index.json',LIB/'scenes/croisement02.rhlos-map.json',LIB/'mission-states/index.json',*[Path(p)for p in routes.values()]]}
 dump(out/'runtime-inputs.json',{**snap,**pins})
 for name in ['editor.tsx','observe-library.js']:shutil.copy2(old/name,out/name)
 shutil.copy2(R/'level-editor/blender/croisement02/native_physical_effect_ui_proof.js',out/'check.js')
 contract=json.loads(Path(req['contract']['path']).read_text());dump(out/'check-config.json',{'entry':req['entry'],'focus':contract['focus_element_id'],'refined':refined,'boundedReceiver':receiver_assets is not None,'receiverAssets':receiver_assets,'cropRegion':{'x':888,'y':900,'width':96,'height':108} if crops else None,'sourceOrigin':contract['native']['origin']})
 run=(old/'run.mjs').read_text().replace(str(old.relative_to(R)),str(out.relative_to(R)))
 run=run.replace('const privateRoutes=new Map();','const privateRoutes=new Map(Object.entries('+json.dumps(routes)+'));')
 run=run.replace("if(p.stage.name!=='canonical-source-camera')throw Error('Unexpected capture');", "if(!/^b05-(physical-over-art|original-art|refined-map|comparison-crop)-tick[0-9]+$/.test(p.stage.name))throw Error('Unexpected capture');")
 run=run.replace("await evaluate('window.scatterCaptures[\"canonical-source-camera\"]=true')", "await evaluate('window.scatterCaptures['+JSON.stringify(p.stage.name)+']=true')")
 run=run.replace("await readFile(join(privateLibrary,path))", "await readFile(privateRoutes.get('/library/'+path)??join(privateLibrary,path))")
 run=run.replace("if(s?.startsWith('FAIL'))throw Error(s);", "await writeFile(join(out,'readiness.json'),JSON.stringify({at:Date.now(),status:s}));if(s?.startsWith('FAIL'))throw Error(s);")
 run=run.replace("await writeFile(join(out,'canonical-checkpoint.json'),JSON.stringify(result,null,2));", "await writeFile(join(out,'canonical-checkpoint.json'),JSON.stringify(result,null,2));const crops=await evaluate('window.nativeEffectCrops??[]');for(const row of crops){if(!/^b05-crop-(original-art|physical-over-art|refined-map)-tick(0|74)$/.test(row.name))throw Error('Unexpected crop');await writeFile(join(out,row.name+'.png'),Buffer.from(row.png.split(',')[1],'base64'));delete row.png;}await writeFile(join(out,'source-pixel-crops.json'),JSON.stringify(crops,null,2));")
 (out/'run.mjs').write_text(run)
 for p in out.iterdir():
  if p.is_file():pins[str(p)]=sha(p)
 dump(out/'inputs.json',pins)
 dump(out/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(out/'run.mjs'),'run_sha256':sha(out/'run.mjs'),'scope':('Bounded receiver subset; full canonical scene proof remains required. ' if receiver_assets is not None else '')+'Actual editor UI physical-effects/native-art prototype, normal frame seek and loop restoration. Original camera. No geometry approval or whole-map completion.','input_request':str(reqpath),'input_sha256':sha(reqpath)})
 print(out)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);p.add_argument('--refined',action='store_true');p.add_argument('--receiver-assets',nargs='*',default=None);p.add_argument('--crops',action='store_true');a=p.parse_args();prepare(a.out.resolve(),a.refined,a.receiver_assets,a.crops)
