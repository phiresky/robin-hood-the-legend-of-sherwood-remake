"""Freeze source-derived full-path vertex references for the real browser player."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from scipy.spatial.transform import Rotation,Slerp
from restart21_butterfly07_geometry_v2 import WING,HINGES
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02';OUT=BASE/'restart67-butterfly07-browser-request-v1'
def main():
 assert not OUT.exists();OUT.mkdir();fitp=BASE/'restart66-butterfly07-global-bright-fit-v1/fit.json';fit=json.loads(fitp.read_text());d=json.loads((BASE/'restart14-butterflies/butterfly07-v3-global-corridor-v2-actual-audit/report.json').read_text())['continuous_height_curve'];depth=CubicSpline(d['times'],d['heights'],bc_type='periodic');s=math.sin(math.radians(35));c=math.cos(math.radians(35));b=np.array([[1,0,0],[0,-s,-c],[0,-c,s]]);convert=np.array([[1,0,0],[0,0,1],[0,-1,0]]);rows=[]
 for tick in range(198):
  t=tick/2;phase=int(t);u=t-phase;a=fit['rows'][phase];z=fit['rows'][(phase+1)%99];pa=np.array(a['parameters']);pb=np.array(z['parameters']);p=pa*(1-u)+pb*u;r=Slerp([0,1],Rotation.from_euler('xyz',[pa[:3],pb[:3]],degrees=True))([u])[0].as_matrix();screen=np.array(a['source']['alpha_centroid_display'])*(1-u)+np.array(z['source']['alpha_centroid_display'])*u;h=float(depth(t));anchor=np.array([screen[0],-(screen[1]+c*h)/s,h]);wings=[]
  for side,sign in enumerate((-1,1)):
   wing=WING.copy();wing[:,0]*=sign;wing=wing@Rotation.from_euler('y',-sign*p[3+side],degrees=True).as_matrix().T;wing[:,0]+=HINGES[side];wings.append(((anchor+(wing@r.T+np.array([p[5],p[6],0]))@b.T)@convert.T).tolist())
  rows.append(dict(tick=tick,wings=wings))
 refs=OUT/'reference.json';refs.write_text(json.dumps(dict(rows=rows,fit_sha256=hashlib.sha256(fitp.read_bytes()).hexdigest()),separators=(',',':'))+'\n');pin=lambda p:dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest());request=dict(status='PRIVATE_UNAPPROVED_GEOMETRY_AND_APPEARANCE',model=pin(BASE/'restart67-butterfly07-fullpath-export-v1/butterfly07-fullpath.glb'),reference=pin(refs),browser_module=pin(ROOT/'level-editor/blender/croisement02/restart67_butterfly07_browser.ts'),catalog_plan=pin(BASE/'restart67-butterfly07-catalog-stage-v1/plan.json'),scope='Real production player full198-tick loop, both wing vertex poses, one-sided material policy, pause/reset/dispose; actual browser isolated views. Not approval or canonical publication.',capture_ticks=[0,4,16,42,54,64,136,176],source_camera_direction=[0,math.sin(math.radians(35)),math.cos(math.radians(35))],browser_started=False,canonical_writes=False);(OUT/'request.json').write_text(json.dumps(request,indent=2)+'\n');print(pin(OUT/'request.json'))
if __name__=='__main__':main()
