"""Freeze own-source butterfly tick references for isolated production review."""
import argparse,json,hashlib,math,importlib.util
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation,Slerp
from scipy.interpolate import CubicSpline
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/active/croisement02'
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--candidate',type=Path,required=True);parser.add_argument('--model',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--number',type=int,required=True);args=parser.parse_args();out=args.output.resolve();out.mkdir(exist_ok=False);p=args.candidate.resolve();fit=json.loads((p/'fit.json').read_text());spec=importlib.util.spec_from_file_location('geo',p/'geometry.py');geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);d=json.loads((p/'inputs.json').read_text())['continuous_height_curve'];curve=CubicSpline(d['times'],d['heights'],bc_type='periodic');s=math.sin(math.radians(35));c=math.cos(math.radians(35));basis=np.array([[1,0,0],[0,-s,-c],[0,-c,s]]);convert=np.array([[1,0,0],[0,0,1],[0,-1,0]]);rows=[];sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();pin=lambda p:dict(path=str(p),sha256=sha(p))
 for tick in range(198):
  phase=tick/2;i=int(phase);t=phase-i;a=fit['rows'][i];z=fit['rows'][(i+1)%99];pa=np.array(a['parameters']);pb=np.array(z['parameters']);q=pa*(1-t)+pb*t;q[:3]=Slerp([0,1],Rotation.from_euler('xyz',[pa[:3],pb[:3]],degrees=True))([t])[0].as_euler('xyz',degrees=True);screen=np.array(a['source']['alpha_centroid_display'])*(1-t)+np.array(z['source']['alpha_centroid_display'])*t;h=float(curve(phase));anchor=np.array([screen[0],-(screen[1]+c*h)/s,h]);_,wings=geo.geometry(q);rows.append(dict(tick=tick,wings=[((anchor+(w+np.r_[q[5:7],0])@basis.T)@convert.T).tolist() for w in wings]))
 ref=out/'reference.json';ref.write_text(json.dumps(dict(rows=rows,fit_sha256=sha(p/'fit.json')),separators=(',',':'))+'\n');row=dict(asset_id=f'croisement02-butterfly{args.number:02d}-private',model=pin(args.model.resolve()),reference=pin(ref),review_scale=24*fit['fixed_uniform_scale'],capture_ticks=[0,4,20,40,64,72,73,74,75,76,98,99,100,101,102,132,134,135,136,137,138,140,176]);(out/'request.json').write_text(json.dumps(dict(status='PRIVATE_UNAPPROVED_GEOMETRY_AND_MOTION',rows=[row],scope='Actual production loader/player at198integer ticks over two loops, including half-native-frame interpolation. No physical contact or source geometry approval.'),indent=2)+'\n')
 old=B/'restart147-butterfly07-current-isolated-browser-v1'
 for name in ['run.mjs','build.mjs']:(out/name).write_text((old/name).read_text().replace(old.name,out.name))
 proof=(B/'restart97-four-butterfly-browser-v2/proof.ts').read_text().replace('restart97_own_butterfly_browser.ts','restart221_butterfly_anatomy_browser.ts');(out/'proof.ts').write_text(proof)
if __name__=='__main__':main()
