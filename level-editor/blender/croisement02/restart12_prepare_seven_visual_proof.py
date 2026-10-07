"""Freeze the seven-state suite with a bounded fixture shell and real-pixel guards."""
import hashlib,json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
expected,name=sys.argv[1:]
subprocess.run([sys.executable,str(Path(__file__).with_name('restart11_prepare_seven_http_proof.py')),expected,name],check=True)
b=ROOT/'level-editor/work/croisement02-refinement/restart2-state'/name
source=Path(__file__).with_name('restart6_verify_remaining_state_editor.mjs')
s=source.read_text()
s="import {spawnSync} from 'node:child_process';\n"+s
s=s.replace("checks.push({name,pass});if(!pass)","checks.push({name,pass});await writeFile(join(out,'progress.json'),JSON.stringify({phase:'assertion',checks:checks.length,name,pass,screenshots:screenshots.length},null,2));if(!pass)")
old="const shot=async name=>{await exec('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');const r=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(out,name+'.png'),Buffer.from(r.data,'base64'));screenshots.push(name+'.png')};"
new=r'''const shot=async name=>{const started=Date.now();await writeFile(join(out,'progress.json'),JSON.stringify({phase:'capture-start',name,checks:checks.length,screenshots:screenshots.length},null,2));await exec('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');const viewport=await exec('window.assertSevenCanvas()');const r=await command('Page.captureScreenshot',{format:'png'}),file=join(out,name+'.png');await writeFile(file,Buffer.from(r.data,'base64'));
 const audit=spawnSync('python3',['-c',"import sys,json;from PIL import Image,ImageStat;d=json.loads(sys.argv[2]);r=d['rect'];im=Image.open(sys.argv[1]).convert('RGB');crop=im.crop((round(r['x'])+8,round(r['y'])+8,round(r['x']+r['width'])-8,round(r['y']+r['height'])-8));small=crop.resize((96,96));colors=len(set(small.getdata()));deviation=max(ImageStat.Stat(small).stddev);chromatic=sum(max(p)-min(p)>20 for p in small.getdata())/(96*96);print(json.dumps({'colors':colors,'max_stddev':deviation,'chromatic_fraction':chromatic}));assert colors>32 and deviation>8 and chromatic>.03,'Blank or near-uniform viewport capture'",file,JSON.stringify(viewport)],{encoding:'utf8'});if(audit.status!==0)throw Error('Visual pixel guard '+name+': '+audit.stderr);const pixels=JSON.parse(audit.stdout);await writeFile(join(out,name+'-canvas-guard.json'),JSON.stringify({status:'PASS',viewport,pixels,elapsed_ms:Date.now()-started},null,2));screenshots.push(name+'.png');await writeFile(join(out,'progress.json'),JSON.stringify({phase:'capture-finished',name,checks:checks.length,screenshots:screenshots.length},null,2))};'''
assert old in s;s=s.replace(old,new)
needle="await check(entry.id+' reset restores native mode',`!window.reviewViewport.stateDelivery.physical.visible&&window.reviewViewport.deliveredStateStatus(${fid}).tick===undefined`);"
assert needle in s
s=s.replace(needle,needle+r'''
 if(entry.id===manifest.entries[0].id){await writeFile(join(out,'early-visual-gate-pending.json'),JSON.stringify({status:'AWAITING_MANUAL_FIRST_NATIVE_INITIAL_APPLIED_REVIEW',images:screenshots},null,2));let accepted=false;for(let i=0;i<1200;i++){try{const gate=await json(join(out,'early-visual-gate.json'));if(gate.status!=='PASS'||gate.images.length!==3)throw Error('Invalid visual gate');for(const row of gate.images){if(!screenshots.includes(row.path)||sha(await readFile(join(out,row.path)))!==row.sha256)throw Error('Visual gate image mismatch')}accepted=true;break; }catch{}await new Promise(r=>setTimeout(r,500));}if(!accepted)throw Error('Manual early visual gate not supplied');}
''')
# The manual gate binds exact saved images; it never conveys asset/user approval.
s=s.replace("const evidence=[];for(const name of screenshots)","const visualGuards=[];for(const name of screenshots){const path=name.replace(/\\.png$/,'-canvas-guard.json');visualGuards.push({path,sha256:sha(await readFile(join(out,path)))})}const earlyGateSha=sha(await readFile(join(out,'early-visual-gate.json')));const evidence=[];for(const name of screenshots)")
s=s.replace("checks,evidence,installed_index_sha256", "checks,evidence,visual_guards:visualGuards,early_visual_gate_sha256:earlyGateSha,installed_index_sha256")
(b/'states.mjs').write_text(s)
p=b/'editor.tsx';p.write_text(p.read_text()+r'''
Object.assign(window,{assertSevenCanvas:()=>{
 const v=window.reviewViewport,c=v.renderer.domElement,r=c.getBoundingClientRect(),gl=v.renderer.getContext(),ratio=devicePixelRatio,w=Math.round(r.width*ratio),h=Math.round(r.height*ratio);
 const proof={rect:{x:r.x,y:r.y,width:r.width,height:r.height},backing:[c.width,c.height],buffer:[gl.drawingBufferWidth,gl.drawingBufferHeight],window:[innerWidth,innerHeight],contextLost:gl.isContextLost(),camera:{left:v.camera.left,right:v.camera.right,top:v.camera.top,bottom:v.camera.bottom},render:{...v.renderer.info.render},mode:v.statePresentationMode};
 if(document.compatMode!=='CSS1Compat'||r.width<200||r.height<200||r.width>innerWidth+1||r.height>innerHeight+1||Math.abs(c.width-w)>1||Math.abs(c.height-h)>1||c.width!==gl.drawingBufferWidth||c.height!==gl.drawingBufferHeight||gl.isContextLost()||!(v.camera.right>v.camera.left))throw Error('Invalid bounded canvas '+JSON.stringify(proof));return proof;
}});
''')
for name in ['run.mjs','capture.mjs']:
 p=b/name;s=p.read_text().replace(source.resolve().as_uri(),(b/'states.mjs').as_uri())
 if name=='run.mjs':
  old='<html><body style="margin:0"><div id="result">STARTING</div><div id="root"></div>'
  new='<!doctype html><html><body style="margin:0"><div id="result" style="position:fixed;bottom:0;left:0;z-index:100">STARTING</div><div id="root" class="editor-app"></div>'
  assert old in s;s=s.replace(old,new)
  s=s.replace(" const stateResult=await states("," await writeFile(join(out,'initial-canvas-guard.json'),JSON.stringify(await evaluate('window.assertSevenCanvas()'),null,2));\n const stateResult=await states(")
  s=s.replace("const exited=new Promise", "chrome.stderr.on('data',data=>writeFile(join(out,'browser.log'),data,{flag:'a'}));\nconst exited=new Promise")
  s=s.replace("await exited;await server.close()", "await exited;await server.close();await writeFile(join(out,'process-final.json'),JSON.stringify({exitCode:chrome.exitCode,signalCode:chrome.signalCode,serverClosed:true},null,2))")
 else:s=s.replace("await writeFile(join(out,name),bytes);views.push", "await writeFile(join(out,name),bytes);await writeFile(join(out,'progress.json'),JSON.stringify({completed:views.length+1,total:16,name},null,2));views.push")
 p.write_text(s)
p=b/'inputs.json';inputs=json.loads(p.read_text())
for path in [b/'states.mjs',b/'editor.tsx',b/'run.mjs',b/'capture.mjs',Path(__file__).resolve()]:inputs['files'][str(path.relative_to(ROOT))]=hashlib.sha256(path.read_bytes()).hexdigest()
inputs['visual_guard_scope']='Bounded CSS/backing/GL canvas on every state capture, actual viewport pixel-content guard, manual first native/initial/applied image gate; all original state assertions retained.'
p.write_text(json.dumps(inputs,indent=2)+'\n');print(b)
