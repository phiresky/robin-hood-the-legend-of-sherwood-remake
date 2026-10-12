"""Prepare complete installed groups 3/4 with native-phase and wood-contact views."""
import argparse,json,hashlib,struct,subprocess,math
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();pin=lambda p:dict(path=str(p.resolve()),sha256=sha(p))
def main():
 p=argparse.ArgumentParser();p.add_argument('--groups',type=int,nargs='+',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);rows=[]
 assert set(a.groups)<= {3,4}
 for index in a.groups:
  source=BASE/f'canopy{index:02}-complete-installed-oct12-v1/report.json';r=json.loads(source.read_text());audit=BASE/f'canopy{index:02}-complete-readback-oct12-v1/report.json';assert json.loads(audit.read_text())['status']=='PASS_COMBINED_HIERARCHY_SURFACE_AND_MORPH_ROUTING';model=Path(r['model']['path']);raw=model.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);del raw;points=[];count=0
  def visit(i,parent):
   nonlocal count
   node=doc['nodes'][i]
   if 'matrix' in node:local=np.array(node['matrix']).reshape(4,4).T
   else:local=np.eye(4);local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
   world=parent@local
   if 'mesh' in node:
    for prim in doc['meshes'][node['mesh']]['primitives']:
     acc=doc['accessors'][prim['attributes']['POSITION']];lo=np.array(acc['min']);hi=np.array(acc['max'])
     if prim.get('targets'):
      count+=1;lo+=np.min([[0,0,0]]+[doc['accessors'][x['POSITION']]['min'] for x in prim['targets']],axis=0);hi+=np.max([[0,0,0]]+[doc['accessors'][x['POSITION']]['max'] for x in prim['targets']],axis=0)
     for x in [lo[0],hi[0]]:
      for y in [lo[1],hi[1]]:
       for z in [lo[2],hi[2]]:points.append((world@np.array([x,y,z,1]))[:3])
   for child in node.get('children',[]):visit(child,world)
  for i in doc['scenes'][doc.get('scene',0)]['nodes']:visit(i,np.eye(4))
  ypoints=np.array(points);zpoints=ypoints[:,[0,2,1]]*np.array([1,-1,1]);center=(zpoints.min(0)+zpoints.max(0))/2;views=[]
  for i in range(8):
   angle=-math.pi/2+i*math.pi/4;direction=np.array([math.cos(angle)*math.cos(math.radians(35)),math.sin(angle)*math.cos(math.radians(35)),math.sin(math.radians(35))]);right=np.cross([0,0,1],direction);right/=np.linalg.norm(right);up=np.cross(direction,right);rotation=np.stack([right,up,direction],axis=1);matrix=np.eye(4);matrix[:3,:3]=rotation;matrix[:3,3]=center+direction*2000;projected=(zpoints-center)@rotation;scale=float(np.abs(projected[:,:2]).max()*2.12);views.append(dict(index=i,camera_matrix_world=matrix.tolist(),ortho_scale=scale))
  vp=a.output/f'group-{index:02}-views.json';vp.write_text(json.dumps(dict(views=views,scope='Original camera direction first, centered on complete group bounds over all poses; no saved camera position inferred from source artwork.'),indent=2)+'\n');native=json.loads((BASE/f'restart80-canopy{index:02}-dense-motion-v1/report.json').read_text())['source_frames'];rows.append(dict(tree_ids=[x['asset_id'].rsplit('-',1)[1] for x in r['assets']],native_source_frames=native,static_sources=[x['static_source'] for x in r['assets']],asset_id=f'canopy-group-{index:02}',primitive_count=count,model=pin(model),placement=dict(dx=0,dy=0,dz=0,rot_deg=0),views=pin(vp.resolve()),export=pin(source),triangle_audit=pin(audit)))
 (a.output/'request.json').write_text(json.dumps(dict(status='PRIVATE_UNAPPROVED_GROUP_MOTION',rows=rows),indent=2)+'\n');template=BASE/'canopy00-complete-review-oct12-v1'
 for name in ['build.mjs','run.mjs','proof.ts']:(a.output/name).write_text((template/name).read_text().replace(template.name,a.output.name))
 proof=a.output/'proof.ts';text=proof.read_text().replace("for(const id of ['43','45','46'])", "for(const id of request.tree_ids)").replace('canopy-group-00-tree-${id}', '${request.asset_id}-tree-${id}');proof.write_text(text)
 runner=a.output/'run.mjs';code=runner.read_text();code="import {appendFileSync} from 'node:fs';\nimport {statfs} from 'node:fs/promises';\nconst reserve=await statfs('.');if(reserve.bavail*reserve.bsize<10*1024**3)throw Error('10GiB disk reserve');const mem=await (await import('node:fs/promises')).readFile('/proc/meminfo','utf8');if(Number(mem.match(/^MemAvailable:\\s+(\\d+)/m)?.[1])<6*1024**2)throw Error('6GiB RAM reserve');\n"+code;code=code.replace("requests.push({path:p,sha256:hash(b),bytes:b.length});","const observation={path:p,sha256:hash(b),bytes:b.length};requests.push(observation);appendFileSync(join(out,'served-responses.jsonl'),JSON.stringify(observation)+'\\n');");runner.write_text(code)
 subprocess.run(['node',str(a.output/'build.mjs')],cwd=ROOT,check=True);subprocess.run(['node','--check',str(a.output/'run.mjs')],cwd=ROOT,check=True);pins=json.loads((a.output/'source-pins.json').read_text())
 for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(a.output/name)]=sha(a.output/name)
 for row in rows:
  for item in row['native_source_frames']+row['static_sources']:
   assert sha(Path(item['path']))==item['sha256'];pins[item['path']]=item['sha256']
  for key in ['model','views','export','triangle_audit']:pins[row[key]['path']]=row[key]['sha256']
 (a.output/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');print(pin(a.output/'run.mjs'))
if __name__=='__main__':main()
