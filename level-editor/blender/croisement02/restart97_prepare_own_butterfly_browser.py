"""Freeze source-derived own-butterfly references for actual browser playback checks."""
import argparse,json,hashlib,importlib.util,subprocess
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from scipy.spatial.transform import Rotation,Slerp
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();pin=lambda p:dict(path=str(p),sha256=sha(p))
def main():
 p=argparse.ArgumentParser();p.add_argument('--numbers',nargs='+',type=int,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);requests=[];s=np.sin(np.radians(35));c=np.cos(np.radians(35));B=np.array([[1,0,0],[0,-s,-c],[0,-c,s]]);E=np.array([[1,0,0],[0,0,1],[0,-1,0]])
 for n in a.numbers:
  candidate=BASE/f'restart83-butterfly{n:02}-contact-candidate-v1';fitp=candidate/'fit.json';fit=json.loads(fitp.read_text());inputs=json.loads((candidate/'inputs.json').read_text());contacts=json.loads((candidate/'contacts/report.json').read_text());assert not any(x['contact_counts'] for x in contacts['results'].values())
  spec=importlib.util.spec_from_file_location('candidate',candidate/'geometry.py');geometry=importlib.util.module_from_spec(spec);spec.loader.exec_module(geometry);curve=inputs['continuous_height_curve'];depth=CubicSpline(curve['times'],curve['heights'],bc_type='periodic');rows=[]
  for tick in range(198):
   t=tick/2;i=int(t);u=t-i;left=fit['rows'][i];right=fit['rows'][(i+1)%99];pa=np.array(left['parameters']);pb=np.array(right['parameters']);parameters=pa*(1-u)+pb*u;parameters[:3]=Slerp([0,1],Rotation.from_euler('xyz',[pa[:3],pb[:3]],degrees=True))([u])[0].as_euler('xyz',degrees=True);screen=np.array(left['source']['alpha_centroid_display'])*(1-u)+np.array(right['source']['alpha_centroid_display'])*u;z=float(depth(t));anchor=np.array([screen[0],-(screen[1]+c*z)/s,z]);body,wings=geometry.geometry(parameters);rows.append(dict(tick=tick,wings=[((anchor+(wing+np.r_[parameters[5:7],0])@B.T)@E.T).tolist() for wing in wings]))
  reference=a.output/f'butterfly{n:02}-reference.json';reference.write_text(json.dumps(dict(rows=rows,fit_sha256=sha(fitp),inputs_sha256=sha(candidate/'inputs.json')),separators=(',',':'))+'\n');model=BASE/f'restart84-butterfly{n:02}-fullpath-v{2 if n==1 else 1}'/f'butterfly{n:02}-fullpath.glb';requests.append(dict(asset_id=f'butterfly{n:02}',model=pin(model),reference=pin(reference.resolve()),fit=pin(fitp),inputs=pin(candidate/'inputs.json'),contacts=pin(candidate/'contacts/report.json'),capture_ticks=[0,4,16,42,54,64,136,176],review_scale=max(24,24*fit['fixed_uniform_scale'])))
 (a.output/'request.json').write_text(json.dumps(dict(status='PRIVATE_UNAPPROVED_OWN_SOURCE_BUTTERFLIES',rows=requests),indent=2)+'\n');template=BASE/'restart88-butterfly07-isolated-browser-v1'
 for name in ['build.mjs','run.mjs']:(a.output/name).write_text((template/name).read_text().replace(template.name,a.output.name))
 proof=(template/'proof.ts').read_text().replace("const request=await(await fetch('./request.json')).json();","const batch=await(await fetch('./request.json')).json();let request:any;const results=[];").replace('const box=new T.Box3()', 'camera.left=-request.review_scale/2;camera.right=request.review_scale/2;camera.top=request.review_scale/2;camera.bottom=-request.review_scale/2;const box=new T.Box3()').replace('`butterfly07-tick-${tick}-view-${index}`','`${request.asset_id}-tick-${tick}-view-${index}`')
 proof=proof.replace(" const result=await verifyButterfly07(request,{phase:async(tick,object)=>{scene.add(object);await capture(tick,object,0);if(tick===0||tick===64)for(let i=1;i<8;i++)await capture(tick,object,i)}});", " for(request of batch.rows){let current:T.Object3D|undefined;const result=await verifyButterfly07(request,{phase:async(tick,object)=>{current=object;scene.add(object);await capture(tick,object,0);if(tick===0||tick===64)for(let i=1;i<8;i++)await capture(tick,object,i)}});if(current)scene.remove(current);results.push({asset_id:request.asset_id,result});}").replace("result,captures,sites:1,controls:[{id:'butterfly07-unapproved-proposal'}]", "results,captures,sites:batch.rows.length,controls:batch.rows.map((r:any)=>({id:r.asset_id}))")
 proof=proof.replace('verifyButterfly07','verifyOwnButterfly').replace('restart67_butterfly07_browser.ts','restart97_own_butterfly_browser.ts')
 (a.output/'proof.ts').write_text(proof);subprocess.run(['node',str(a.output/'build.mjs')],cwd=ROOT,check=True);subprocess.run(['node','--check',str(a.output/'run.mjs')],cwd=ROOT,check=True);pins=json.loads((a.output/'source-pins.json').read_text())
 for name in ['request.json','proof.ts','proof.js','index.html','run.mjs','build.mjs']:pins[str(a.output/name)]=sha(a.output/name)
 for r in requests:
  for key in ['model','reference','fit','inputs','contacts']:pins[r[key]['path']]=r[key]['sha256']
 (a.output/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');print(len(requests),pin(a.output/'run.mjs'))
if __name__=='__main__':main()
