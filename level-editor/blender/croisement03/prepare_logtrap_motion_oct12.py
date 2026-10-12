"""Freeze every visible trap component frame at its shared native placement."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=WORK/'motion-source-v1';out.mkdir(exist_ok=False)
 source=WORK/'source-qualification.json';timing=WORK/'timing.json';q=json.loads(source.read_text());t=json.loads(timing.read_text());frames=[];pins={str(source):sha(source),str(timing):sha(timing)}
 for target in q['targets']:
  profile=target['profile'];row=next(r for r in target['rows'] if r['action_id']==160)
  for i,f in enumerate(row['frames']):
   path=ROOT/f['source_path'];assert sha(path)==f['sha256'];pins[str(path)]=sha(path);a=np.array(Image.open(path).convert('RGBA'));rgb=a[:,:,:3].astype(np.uint16);packed=((rgb[:,:,0]>>3)<<11)|((rgb[:,:,1]>>2)<<5)|(rgb[:,:,2]>>3);a[(packed==0x07c0)|(a[:,:,3]<128),3]=0
   # Shadow markers are retained in their own source role, never opaque wood.
   shadow=(packed==0x001f)&(a[:,:,3]>=128);a[shadow,3]=0
   im=Image.new('RGBA',(300,240));im.alpha_composite(Image.fromarray(a),(round(469+f['offset_x']-420),round(424+f['offset_y']-380)))
   name=f'{profile.rsplit("_",1)[-1]}-{i:02}.png';im.save(out/name);updates=[r['updates_since_reset'] for r in t['timelines'][profile] if r['frame']==i];frames.append({'profile':profile,'frame':i,'path':str(out/name),'sha256':sha(out/name),'source_sha256':sha(path),'updates':updates,'opaque_wood_pixels':int((np.array(im)[:,:,3]>=128).sum()),'shadow_marker_pixels':int(shadow.sum()),'native_origin':[469,424],'offset':[f['offset_x'],f['offset_y']],'delay':f['delay']})
 visible=sorted((f for f in frames if f['opaque_wood_pixels']),key=lambda f:f['updates'][0]);assert len(visible)==30,len(visible)
 sheet=Image.new('RGB',(1200,((len(visible)+3)//4)*270),'#303030');d=ImageDraw.Draw(sheet)
 for n,f in enumerate(visible):
  x=n%4*300;y=n//4*270;im=Image.open(f['path']);sheet.paste(im,(x,y),im);d.text((x+5,y+240),f'{f["profile"].rsplit("_",1)[-1]}:{f["frame"]} updates {min(f["updates"])}–{max(f["updates"])}',fill='white')
 sheet.save(out/'all-visible-native-frames.png');report={'status':'SOURCE_ONLY_NO_RIG','frames':frames,'pins':pins,'scope':['All frames share exact native art origin and crop; no independent recentering.','Update indices are local increments after reset, not mission global ticks.','Source b1/b2/b3 role transitions are exact; no per-log temporal identities inferred.','Shadow RGB565001f is separate from wood; transparent07c0 cannot become geometry.']};(out/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(len(visible),'visible frame states frozen')
if __name__=='__main__':main()
