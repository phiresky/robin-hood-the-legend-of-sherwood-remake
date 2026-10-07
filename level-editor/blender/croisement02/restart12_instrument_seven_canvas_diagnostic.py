"""Add private before/after shell-layout diagnostics without changing Editor runtime."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
base=Path(sys.argv[1]).resolve()
assert base.parent==ROOT/'level-editor/work/croisement02-refinement/restart2-state'
fixture=base/'editor.tsx'
fixture.write_text(fixture.read_text()+r'''
import * as THREE from "three";
Object.assign(window,{sevenCanvasDiagnostic:async(fixShell=false)=>{
 if(fixShell){document.querySelector('#root').classList.add('editor-app');Object.assign(document.querySelector('#result').style,{position:'fixed',bottom:'0',left:'0',zIndex:'100'});await new Promise(r=>setTimeout(r,1000));}
 const v=window.reviewViewport,renderer=v.renderer,canvas=renderer.domElement,gl=renderer.getContext();
 const describe=el=>{if(!el)return null;const r=el.getBoundingClientRect(),s=getComputedStyle(el);return{tag:el.tagName,className:el.className,rect:{x:r.x,y:r.y,width:r.width,height:r.height},client:[el.clientWidth,el.clientHeight],scroll:[el.scrollWidth,el.scrollHeight],backing:el instanceof HTMLCanvasElement?[el.width,el.height]:null,style:{display:s.display,position:s.position,height:s.height,width:s.width,flex:s.flex,overflow:s.overflow}}};
 const report={fixShell,compatMode:document.compatMode,viewport:[innerWidth,innerHeight],elements:['html','body','#root','.editor','.editor-body','.editor-canvas'].map(q=>({selector:q,...describe(document.querySelector(q))})),canvases:[...document.querySelectorAll('canvas')].map(describe),renderer:{contextLost:gl.isContextLost(),drawingBuffer:[gl.drawingBufferWidth,gl.drawingBufferHeight],info:{memory:{...renderer.info.memory},render:{...renderer.info.render},programs:renderer.info.programs?.length},canvas:describe(canvas)},camera:{target:v.orbit?.target?.toArray(),frustum:v.frustum,type:v.camera.type,position:v.camera.position.toArray(),up:v.camera.up.toArray(),zoom:v.camera.zoom,near:v.camera.near,far:v.camera.far,left:v.camera.left,right:v.camera.right,top:v.camera.top,bottom:v.camera.bottom,matrixWorld:v.camera.matrixWorld.elements,projection:v.camera.projectionMatrix.elements},scene:{children:v.scene.children.length,parts:v.partViews.size,visibleParts:[...v.partViews.values()].filter(p=>p.wrapper.visible).length},presentation:v.statePresentationMode};
 renderer.render(v.scene,v.camera);report.renderer.afterDirectRender=JSON.parse(JSON.stringify(renderer.info.render));const pixels=new Uint8Array(4),samples=[];for(const x of [.25,.5,.75])for(const y of [.25,.5,.75]){gl.readPixels(Math.floor(gl.drawingBufferWidth*x),Math.floor(gl.drawingBufferHeight*y),1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixels);samples.push([...pixels])}report.samples=samples;report.directPng=canvas.toDataURL('image/png');return report;
}});
''')
runner=base/'run.mjs';s=runner.read_text();marker=" const stateResult=await states("
pos=s.index(marker)
end=s.index(" await verifyPins();",pos)
replacement=""" for(const [label,fix]of [['before-shell',false],['after-shell',true]]){const d=await evaluate('window.sevenCanvasDiagnostic('+fix+')');await writeFile(join(out,label+'-canvas.png'),Buffer.from(d.directPng.split(',')[1],'base64'));delete d.directPng;await writeFile(join(out,label+'.json'),JSON.stringify(d,null,2));const shot=await command('Page.captureScreenshot',{format:'png'});await writeFile(join(out,label+'-page.png'),Buffer.from(shot.data,'base64'));}
 await writeFile(join(out,'diagnostic-complete.json'),JSON.stringify({status:'DIAGNOSTIC_ONLY_NO_STATE_SUITE',scope:'Before/after private shell layout. All runtime/source files unchanged.'},null,2));
"""
s=s[:pos]+replacement+s[end:]
start=s.index(" await writeFile(join(out,'verification.json')")
end=s.index("}catch(error)",start)
s=s[:start]+" console.log('Canvas diagnostic complete; full state suite not run');\n"+s[end:]
s=s.replace("await exited;await server.close()", "await exited;await server.close();await writeFile(join(out,'process-final.json'),JSON.stringify({exitCode:chrome.exitCode,signalCode:chrome.signalCode,serverClosed:true},null,2))")
runner.write_text(s)
p=base/'inputs.json';v=json.loads(p.read_text())
for path in [fixture,runner,Path(__file__).resolve()]:v['files'][str(path.relative_to(ROOT))]=hashlib.sha256(path.read_bytes()).hexdigest()
v['scope']='Diagnostic-only private shell before/after; no state suite proof.';p.write_text(json.dumps(v,indent=2)+'\n')
print(base)
