"""Freeze net01 source art, script coupling and distinct endpoint outline fits."""
import hashlib,json,math,pathlib
import numpy as np
from PIL import Image
from scipy.optimize import least_squares
R=pathlib.Path(__file__).resolve().parents[3];P=R/'level-editor/work/croisement01-refinement/mission-endpoint-preparation-oct12-v1';O=R/'level-editor/work/croisement01-refinement/mission-net01-oct12-v1'
def sha(p):return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));F=np.array([0,.1,.25,.4,.55,.7,.85,1.])
OUTLINES={'e':[(40,137),(48,144),(55,157),(59,170),(58,181),(53,194),(48,203),(40,205),(34,201),(29,190),(30,177),(33,162),(36,148)],'i':[(41,137),(49,145),(58,155),(67,169),(72,185),(70,199),(61,209),(47,215),(31,214),(20,208),(14,199),(13,187),(16,173),(23,159),(32,147)]}
def vertices(p,ratio):
 cx,bottom,height,tilt,*rs=p;angles=np.arange(48)*math.tau/48;return np.array([[cx+tilt*(t-.5)+r*math.cos(a),-bottom/SIN+ratio*r*math.sin(a),t*height] for t,r in zip(F,[.15,*rs,.15]) for a in angles])
def main():
 O.mkdir(exist_ok=True);d=json.loads((P/'inventory.json').read_text());sources={};fits={};directions=np.column_stack((np.cos(np.arange(96)*math.tau/96),np.sin(np.arange(96)*math.tau/96)))
 for suffix in 'gei':
  p=next(x for x in d['patch_families'] if x['profile']=='Croisement01 - piege01'+suffix);f=p['frames']['final'][0];sources[suffix]={'path':str(P/f['path']),'sha256':f['sha256'],'offset':f['offset'],'size':f['size'],'position':p['instances'][0]['display_position'],'all_final_frames':p['frames']['final'],'instances':p['instances']}
  if suffix=='g':continue
  outline=np.array(OUTLINES[suffix],float);cx=outline[:,0].mean();bottom=outline[:,1].max();height=np.ptp(outline[:,1])/COS;width=np.ptp(outline[:,0]);ratio=.7 if suffix=='e' else 1.;initial=np.r_[cx,bottom,height,0,np.sin(F[1:-1]*np.pi)**.8*width/2];target=(outline@directions.T).max(axis=0)
  def residual(params):
   v=vertices(params,ratio);pr=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS));return np.r_[(pr@directions.T).max(axis=0)-target,.08*np.diff(params[4:],n=2)]
  fit=least_squares(residual,initial,bounds=(np.r_[cx-10,bottom-12,height*.6,-12,np.repeat(.1,6)],np.r_[cx+10,bottom+12,height*1.4,12,np.repeat(width*.8,6)]),max_nfev=500);assert fit.success;fits[suffix]={'parameters':fit.x.tolist(),'fractions':F.tolist(),'depth_ratio':ratio,'outline':OUTLINES[suffix],'rms':float(np.sqrt(np.mean(residual(fit.x)[:96]**2))),'max':float(np.abs(residual(fit.x)[:96]).max())}
 t=next(x for x in d['target_profiles'] if x['profile']=='Croisement01 - piege01h');f=t['rows'][0]['frames'][0];sources['initial']={'path':str(P/f['path']),'sha256':f['sha256'],'offset':[f['offset_x'],f['offset_y']],'size':f['size'],'position':[35,265]}
 raw_initial=pathlib.Path(sources['initial']['path']);rgba=np.array(Image.open(raw_initial).convert('RGBA'));key=np.all(rgba[:,:,:3]==[0,251,0],axis=2);rgba[key,3]=0;normalized=O/'initial-native-alpha.png';Image.fromarray(rgba).save(normalized);sources['initial'].update(raw_path=str(raw_initial),raw_sha256=sha(raw_initial),path=str(normalized),sha256=sha(normalized),alpha_conversion={'key_rgb':[0,251,0],'pixels':int(key.sum()),'known_rgb_preserved':True})
 script=R/'datadirs/fullgame_gog_hackable/Data/Levels/Emb01_FoA_EC.scb.ts';text=script.read_text();begin=text.index('class cible_filet01_');end=text.index('\nclass ',begin+1);fragment=text[begin:end];assert 'GetPatchScript(/*iPosition*/ 8)' in fragment and 'GetPatchScript(/*iPosition*/ 7)' in fragment and 'GetPatchScript(/*iPosition*/ 6)' in fragment
 (O/'script-coupling.txt').write_text(fragment+'\n')
 packet={'scope':'Private net01 endpoint geometry preparation; Emb01 placement authority; all six missions retain separate source contracts.','sources':sources,'fits':fits,'source_inventory_sha256':sha(P/'inventory.json'),'script_sha256':sha(script),'coupling':{'mission':'Emb01_FoA_EC','global1_occupied':'patch8=mission2 piege01i plus patch6=mission0 ground','global1_empty':'patch7=mission1 piege01e plus patch6=mission0 ground','target':'Actor59 action160 hides initial support appearance; actor66 action210 is trigger-target animation, separate scope.','ordering':'Six map patches precede mission patches; zone tracks actor occupancy in global1.','limitation':'Only representative mission script checked; no engine execution or other-mission parity claim.'},'source_camera':{'elevation_deg':35,'ray':[0,-COS,SIN],'screen_projection':'x=X, y=-Y*sin35-Z*cos35'},'inference':['Bag depth/hidden back are inferred; e and i fit independently.','Endpoint phase0 only;14 final wobble frames remain explicit future motion scope.','Rope upper source ends are retained; actual neighboring-tree attachment/contact is not proven by standalone endpoint.']}
 for s in sources.values():assert sha(s['path'])==s['sha256']
 (O/'source.json').write_text(json.dumps(packet,indent=2)+'\n');print({k:(v['rms'],v['max']) for k,v in fits.items()})
if __name__=='__main__':main()
