"""Prepare after publication only; never launch or modify canonical resources."""
from pathlib import Path
import argparse,hashlib,json,shutil
from audit_smoke_catalog_authority import published_authority
R=Path.cwd();B=R/'level-editor/work/active/croisement02/approved-round-wood-integration-oct11-v1';OLD=B/'editor-check-v3';LIB=R/'level-editor/library'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
def prepare(plan_path,out):
 plan=json.loads(plan_path.read_text())
 catalog_audit=published_authority(plan,LIB)
 state_packet=R/'level-editor/work/active/croisement02/restart136-installed-state-ui-inputs-v1/inputs.json';states=json.loads(state_packet.read_text())
 for pin in states['pins']:assert sha(Path(pin['path']))==pin['sha256'],pin['path']
 out.mkdir(exist_ok=False)
 dump(out/'current-catalog-audit.json',catalog_audit)
 authority=json.loads((OLD/'runtime-authority.json').read_text());snapshot={}
 for d in authority['sourceDirectories']:
  for p in sorted((R/d).rglob('*')):
   if p.is_file():
    rel=str(p.relative_to(R));snapshot[rel]=sha(p)
    if p.suffix in ('.ts','.tsx','.js','.jsx','.mjs','.cjs','.css','.json'):
     dst=out/'snapshot'/rel;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dst)
 authority['membership']=sorted(snapshot);authority['index']={'path':str(LIB/'3d-assets/index.json'),'initial_sha256':sha(LIB/'3d-assets/index.json')}
 dump(out/'source-snapshot.json',snapshot);dump(out/'runtime-authority.json',authority)
 inputs={str(plan_path):sha(plan_path),str(state_packet):sha(state_packet)}
 for rec in plan['records']:inputs[rec['target']]=catalog_audit['current_index_sha256'] if Path(rec['target'])==LIB/'3d-assets/index.json' else rec['new_sha256']
 for row in catalog_audit['changes']:inputs[row['descriptor']['path']]=row['descriptor']['sha256']
 inputs[str(out/'current-catalog-audit.json')]=sha(out/'current-catalog-audit.json')
 for pin in states['pins']:inputs[pin['path']]=pin['sha256']
 runtime=dict(snapshot);runtime.update(inputs);dump(out/'runtime-inputs.json',runtime)
 shutil.copy2(OLD/'observe-library.js',out/'observe-library.js');shutil.copy2(OLD/'editor.tsx',out/'editor.tsx')
 check=R/'level-editor/blender/croisement02/canonical_round_wood_smoke.js';shutil.copy2(check,out/'check.js')
 cfg=json.loads((OLD/'check-config.json').read_text());cfg['states']=[dict(row,kind='climbing')for row in json.loads((R/'level-editor/work/active/croisement02/restart79-climbing-tree42-ui-browser-v1/climbing-config.json').read_text())['rows']];cfg['states'].append(dict(json.loads((R/'level-editor/work/active/croisement02/restart79-climbing-tree42-ui-browser-v1/tree42-config.json').read_text()),kind='tree42'));dump(out/'check-config.json',cfg)
 old=str(OLD.relative_to(R));new=str(out.relative_to(R));run=(OLD/'run-v4.mjs').read_text().replace(old,new).replace(new+'/browser-v4',new+'/browser-v1').replace(str((B/'publication/stage-v3-current/library').relative_to(R)),str(LIB.relative_to(R)))
 start=run.index(' const statePacket=');end=run.index(' await Promise.all(modulePending);',start)
 run=run[:start]+''' await writeFile(join(out,'canonical-map-ready.json'),JSON.stringify(await evaluate('({groups:window.reviewViewport.bindings.document().groups.length,sources:window.reviewViewport.bindings.document().assetSources})'),null,2));
 await evaluate('window.canonicalSmoke='+await readFile(CONFIG,'utf8')+';window.scatterCaptures={};window.scatterDone=false');
 await evaluate('void ('+await readFile(CHECK,'utf8')+').then(r=>{window.scatterResult=r;window.scatterDone=true},e=>{window.scatterError=String(e);window.scatterDone=true})');
 let result;
 for(let i=0;i<1800;i++){await new Promise(r=>setTimeout(r,250));const p=await evaluate('({done:window.scatterDone,error:window.scatterError,stage:window.scatterStage,captures:window.scatterCaptures,evidence:window.canonicalSmokeEvidence})');await writeFile(join(out,'progress.json'),JSON.stringify(p,null,2));if(p.error)throw Error(p.error);if(p.stage?.name&&!p.captures[p.stage.name]){if(p.stage.name!=='canonical-source-camera')throw Error('Unexpected capture');const png=await command('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(out,p.stage.name+'.png'),Buffer.from(png.data,'base64'));await evaluate('window.scatterCaptures["canonical-source-camera"]=true')}if(p.done){result=await evaluate('window.scatterResult');break}}
 if(result?.status!=='SCATTER_EXECUTED_PENDING_VISUAL_REVIEW')throw Error('Canonical smoke incomplete');await writeFile(join(out,'canonical-checkpoint.json'),JSON.stringify(result,null,2));
'''.replace('CONFIG',json.dumps(str(out/'check-config.json'))).replace('CHECK',json.dumps(str(out/'check.js')))+run[end:]
 run=run.replace('i<3600','i<2200').replace("if(chrome.exitCode===0)await rm(profile,{recursive:true,force:false});",'/* Preserve profile and evidence. */')
 # Use the completed retained-profile proof's lighter UI capture configuration.
 run=run.replace('--window-size=1440,1000','--window-size=960,720').replace("format:'png',captureBeyondViewport:false", "format:'png',captureBeyondViewport:false,fromSurface:false")
 # Retain resource evidence even if an external supervisor ends the namespace.
 run=run.replace('appendFileSync, mkdirSync','appendFileSync, mkdirSync, readFileSync, readdirSync')
 run=run.replace("const hostEvent=(event)=>", """const resourceSample=()=>{
  const read=p=>{try{return readFileSync(p,'utf8')}catch{return null}};
  const mem=read('/proc/meminfo')??'';
  const processes=readdirSync('/proc').filter(n=>/^\\d+$/.test(n)).map(pid=>{
   const status=read('/proc/'+pid+'/status')??'';
   return {pid:Number(pid),name:status.match(/^Name:\\s+(.+)$/m)?.[1],rssKiB:Number(status.match(/^VmRSS:\\s+(\\d+)/m)?.[1]??0)};
  }).sort((a,b)=>b.rssKiB-a.rssKiB).slice(0,12);
  return {availableKiB:Number(mem.match(/^MemAvailable:\\s+(\\d+)/m)?.[1]??0),swapFreeKiB:Number(mem.match(/^SwapFree:\\s+(\\d+)/m)?.[1]??0),pressure:read('/proc/pressure/memory'),cgroupEvents:read('/sys/fs/cgroup/memory.events'),processes};
 };
 const hostEvent=(event)=>""")
 run=run.replace('memory:process.memoryUsage()', 'memory:process.memoryUsage(),resources:resourceSample()')
 run=run.replace("chrome.stderr.on('data',d=>logs=(logs+d.toString()).slice(-20000));", "chrome.stderr.on('data',d=>{logs=(logs+d.toString()).slice(-20000);appendFileSync(join(out,'chrome-stderr-live.log'),d)});")
 # Scene listing is generated by the canonical mount; match its exact current encoding.
 run=run.replace("path!=='scenes/index.json'&&path!=='3d-assets/index.json'&&actual!==","actual!==")
 run=run.replace("update(await readFile(join(privateLibrary,path)))", "update(path==='scenes/index.json'?Buffer.from(JSON.stringify((await readdir(join(privateLibrary,'scenes'))).filter(n=>n.endsWith('.rhlos-map.json')).sort((a,b)=>a.localeCompare(b)))+'\\n'):await readFile(join(privateLibrary,path)))")
 (out/'run.mjs').write_text(run)
 for f in ['editor.tsx','check.js','check-config.json','observe-library.js','runtime-authority.json','source-snapshot.json']:inputs[str(out/f)]=sha(out/f)
 dump(out/'inputs.json',inputs)
 dump(out/'request.json',{'status':'PREPARED_NOT_LAUNCHED','run':str(out/'run.mjs'),'run_sha256':sha(out/'run.mjs'),'plan':str(plan_path),'plan_sha256':sha(plan_path),'current_catalog_audit':{'path':str(out/'current-catalog-audit.json'),'sha256':sha(out/'current-catalog-audit.json')},'runtime':'Fresh current source snapshot at postpublication preparation; independent of prior private proof.','scope':'Actual canonical source map and selected state resources; normal map/group selection, source camera, controls. No save/reload or repeated clock claim.'})
 for path,digest in inputs.items():assert sha(Path(path))==digest,'Input drift during preparation: '+path
 print(out)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--plan',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args();prepare(a.plan.resolve(),a.out.resolve())
