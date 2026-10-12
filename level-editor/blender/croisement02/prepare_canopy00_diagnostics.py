"""Prepare explicit diagnostic materials and calibrated native-plane views privately."""
from pathlib import Path
import json,hashlib,subprocess
R=Path.cwd();B=R/'level-editor/work/active/croisement02';OLD=B/'canopy00-complete-review-oct12-v1';O=B/'canopy00-junction-diagnostics-oct12-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert not (O/'handoff.json').exists();O.mkdir(exist_ok=True)
request=json.loads((OLD/'request.json').read_text());request['scope']='Supplemental diagnostic-only materials and source-bbox camera; original34appearance renders remain authoritative appearance evidence.';(O/'request.json').write_text(json.dumps(request,indent=2)+'\n')
for name in ['run.mjs','build.mjs']:(O/name).write_text((OLD/name).read_text().replace(OLD.name,O.name))
s=(OLD/'proof.ts').read_text();a=s.index(' async function contact(');z=s.index(' for(request of batch.rows)',a)
s=s[:a]+'''
 async function emit(name:string){renderer.render(scene,camera);w.pendingCapture={name,data:renderer.domElement.toDataURL('image/png')};for(let n=0;n<2400&&!w.captureAck[name];n++)await new Promise(r=>setTimeout(r,50));if(!w.captureAck[name])throw Error('Capture timeout '+name);w.pendingCapture=null;captures.push(name)}
 async function nativeView(tick:number,object:T.Object3D){
  const [left,top,width,height]=batch.native_source_frames[0].bbox;const sin=Math.sin(35*Math.PI/180),cos=Math.cos(35*Math.PI/180);const target=new T.Box3().setFromObject(object).getCenter(new T.Vector3()),sourceY=target.z*sin-target.y*cos,delta=top+height/2-sourceY;target.x=left+width/2;target.y-=delta*cos;target.z+=delta*sin;
  camera.position.copy(target).add(new T.Vector3(0,sin,cos).multiplyScalar(2000));camera.up.set(0,1,0);camera.lookAt(target);camera.left=-width/2;camera.right=width/2;camera.top=height/2;camera.bottom=-height/2;camera.near=.01;camera.far=5000;camera.updateProjectionMatrix();camera.updateMatrixWorld(true);renderer.setSize(width,height);await emit(`canopy00-native-bbox-tick-${tick}`);
 }
 async function contact(tick:number,object:T.Object3D){
  const groups:T.Object3D[]=[];object.traverse(o=>{if(/placed.?root/.test(o.name))groups.push(o)});renderer.setSize(512,512);renderer.localClippingEnabled=true;
  for(const id of ['43','45','46']){
   const group=groups.find(o=>o.name.includes('tree-'+id));if(!group)throw Error('Missing tree '+id);const visibility=groups.map(g=>g.visible);groups.forEach(g=>g.visible=g===group);
   const wood=new T.Box3();group.traverse(o=>{const m=o as T.Mesh;if(m.isMesh&&!m.morphTargetInfluences?.length)wood.union(new T.Box3().setFromObject(m))});if(wood.isEmpty())throw Error('Missing wood');const target=wood.getCenter(new T.Vector3());target.y=wood.max.y-10;
   for(const [angle,label]of [[0,'original'],[Math.PI,'reverse']] as const){
    const elevation=35*Math.PI/180,dir=new T.Vector3(Math.sin(angle)*Math.cos(elevation),Math.sin(elevation),Math.cos(angle)*Math.cos(elevation));camera.position.copy(target).addScaledVector(dir,1000);camera.up.set(0,1,0);camera.lookAt(target);camera.left=-110;camera.right=110;camera.top=110;camera.bottom=-110;camera.near=.01;camera.far=4000;camera.updateProjectionMatrix();camera.updateMatrixWorld(true);
    for(const mode of ['transparent','cutaway']){
     const materials:{mesh:T.Mesh;prior:T.Material|T.Material[];diagnostic:T.Material}[]=[];group.traverse(o=>{const mesh=o as T.Mesh;if(!mesh.isMesh)return;const crown=!!mesh.morphTargetInfluences?.length,diagnostic=new T.MeshBasicMaterial({color:crown?0x30cbd4:0xf030bc,side:T.DoubleSide,transparent:crown&&mode==='transparent',opacity:crown&&mode==='transparent'?.20:1,depthWrite:!(crown&&mode==='transparent'),clippingPlanes:crown&&mode==='cutaway'?[new T.Plane().setFromNormalAndCoplanarPoint(dir.clone().negate(),target)]:[]});materials.push({mesh,prior:mesh.material,diagnostic});mesh.material=diagnostic});
     try{await emit(`canopy00-tree-${id}-${mode}-${label}-tick-${tick}`)}finally{for(const m of materials){m.mesh.material=m.prior;m.diagnostic.dispose()}}
    }
   }
   groups.forEach((g,i)=>g.visible=visibility[i]);
  }
  renderer.localClippingEnabled=false;
 }
''' +s[z:]
s=s.replace('await capture(tick,object,0);if(tick===0||tick===28)await contact(tick,object);if(tick===28)for(let i=1;i<8;i++)await capture(tick,object,i)','await nativeView(tick,object);if(tick===0||tick===28)await contact(tick,object);')
assert 'await nativeView(tick,object)'in s
start=s.index("scope:'Actual production loader/player113ticks");end=s.index("'};",start);s=s[:start]+"scope:'Supplemental diagnostic-only:15exact native-bbox phase views and24isolated role-color/cutaway junction views. Magenta fixedwood,cyan movingcrown; no appearance replacement or contact certification. All motion unapproved.'"+s[end+1:]
(O/'proof.ts').write_text(s)
subprocess.run(['node',str(O/'build.mjs')],check=True);subprocess.run(['node','--check',str(O/'run.mjs')],check=True)
pins=json.loads((O/'source-pins.json').read_text());prior=json.loads((OLD/'source-pins.json').read_text());pins.update(prior)
for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(O/name)]=sha(O/name)
(O/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n')
(O/'handoff.json').write_text(json.dumps({'status':'PREPARED_NOT_LAUNCHED','runner':{'path':str(O/'run.mjs'),'sha256':sha(O/'run.mjs')},'pins_sha256':sha(O/'source-pins.json'),'captures':39,'legend':'Diagnostic magenta=fixed wood, cyan=moving crown. Transparent20% and half-space cutaway remove visual occlusion only; no source texture/material edits. Other trees hidden only for diagnostic captures.','native_camera':'Exact native bbox416x335, X=worldX and imageY=worldZ*sin35-worldY*cos35; source pixel scale1:1.15phase frames inclclosure.','direction_check':'Original contact [0,sin35,cos35] equals views0 transformed Zup→Yup direction within5.1e-17. Reverse rotates azimuth180degrees retaining35degree elevation.','limits':'Diagnostics alone do not certify intersections/attachment. All inferred motion unapproved. Original34appearance captures preserved.'},indent=2)+'\n');print(O)
