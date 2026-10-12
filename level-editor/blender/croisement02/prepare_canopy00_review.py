"""Prepare a bounded first combined-canopy proof with explicit contact evidence."""
from pathlib import Path
import json,hashlib
R=Path.cwd();B=R/'level-editor/work/active/croisement02';OLD=B/'restart101-five-combined-canopy-browser-v1';O=B/'canopy00-complete-review-oct12-v1'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb')as f:
  for c in iter(lambda:f.read(1048576),b''):h.update(c)
 return h.hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
O.mkdir(exist_ok=False);old=str(OLD.relative_to(R));new=str(O.relative_to(R))
req=json.loads((OLD/'request.json').read_text());req['rows']=req['rows'][:1];req['scope']='Only group00, trees43/45/46. All inferred motion unapproved; no extrapolation to other31crowns.'
source=json.loads((B/'restart80-canopy00-dense-motion-v1/report.json').read_text());req['native_source_frames']=source['source_frames'];req['contact_scope']='Twelve original/reverse crown-to-fixed-wood junction captures at basis and phase7; visual review required, not certified no-intersection.'
for row in req['rows']:
 for k in ('model','views','export','triangle_audit'):
  p=Path(row[k]['path']);p=p if p.is_absolute()else R/p;assert sha(p)==row[k]['sha256'];row[k]['path']=str(p)
for x in req['native_source_frames']:assert sha(Path(x['path']))==x['sha256']
dump(O/'request.json',req)
for f in ('run.mjs','build.mjs'):(O/f).write_text((OLD/f).read_text().replace(old,new))
s=(OLD/'proof.ts').read_text()
insert="""
 async function contact(tick:number,object:T.Object3D){
  for(const id of ['43','45','46']){
   let group:T.Object3D|undefined;object.traverse(o=>{if(o.name.includes('tree-'+id)&&/placed.?root/.test(o.name))group=o});if(!group)throw Error('Missing placed group '+id);
   const wood=new T.Box3();group.traverse(o=>{const mesh=o as T.Mesh;if(mesh.isMesh&&!mesh.morphTargetInfluences?.length)wood.union(new T.Box3().setFromObject(mesh))});if(wood.isEmpty())throw Error('Missing fixed wood '+id);
   const target=wood.getCenter(new T.Vector3());target.y=wood.max.y-10;const elevation=35*Math.PI/180;
   for(const [angle,label]of [[0,'original'],[Math.PI,'reverse']] as const){
    camera.position.copy(target).add(new T.Vector3(Math.sin(angle)*Math.cos(elevation),Math.sin(elevation),Math.cos(angle)*Math.cos(elevation)).multiplyScalar(1000));camera.up.set(0,1,0);camera.lookAt(target);camera.left=-100;camera.right=100;camera.top=100;camera.bottom=-100;camera.near=.01;camera.far=4000;camera.updateProjectionMatrix();camera.updateMatrixWorld(true);renderer.render(scene,camera);
    const name=`canopy-group-00-tree-${id}-contact-${label}-tick-${tick}`;w.pendingCapture={name,data:renderer.domElement.toDataURL('image/png')};for(let n=0;n<2400&&!w.captureAck[name];n++)await new Promise(r=>setTimeout(r,50));if(!w.captureAck[name])throw Error('Contact capture timeout');w.pendingCapture=null;captures.push(name);
   }
  }
 }
"""
s=s.replace(' for(request of batch.rows)',insert+' for(request of batch.rows)');s=s.replace('await capture(tick,object,0);if(tick===28)', 'await capture(tick,object,0);if(tick===0||tick===28)await contact(tick,object);if(tick===28)');s=s.replace('no source parity or neighbor-contact claim.','native phase reference files separately pinned; crown-to-wood contact captures require visual review. No certified self-intersection or external-neighbor clearance claim.')
(O/'proof.ts').write_text(s)
print(O)
