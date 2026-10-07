"""Measure native link phase before imposing any physical return hypothesis."""
import hashlib,json,math,statistics
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';SOURCE=WORK/'geometry-pass-01/native-state-source-v1';OUT=WORK/'restart2/winch-chain-native-phase-v1.json'
if OUT.exists():raise FileExistsError(OUT)
record=next(r for r in json.loads((SOURCE/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((WORK/'restart2/winch-motion-physical-v2/motion.json').read_text());all_profiles=[]
for i,(frame,pose) in enumerate(zip(frames,motion['rows'])):
 im=Image.open(SOURCE/frame['image']).convert('RGBA');profiles={}
 for side,lo,hi in [('left',2394,2405),('right',2406,2415)]:
  profile=[]
  for y in range(884,931):
   if side=='left' and abs(y+.5-pose['screen_center_y'])<12:continue
   count=sum(im.getpixel((x-frame['bbox'][0],y-frame['bbox'][1]))[3]/255 for x in range(lo,hi) if 0<=x-frame['bbox'][0]<im.width and 0<=y-frame['bbox'][1]<im.height);profile.append([y,count])
  profiles[side]=profile
 all_profiles.append(profiles)
templates={side:[statistics.mean(v for y,v in all_profiles[0][side] if y%7==i) for i in range(7)] for side in ['left','right']}
def score(profile,template,phase):
 observed=[v for _,v in profile];predicted=[]
 for y,_ in profile:
  t=(y-phase)%7;i=int(t);f=t-i;predicted.append(template[i]*(1-f)+template[(i+1)%7]*f)
 om=statistics.mean(observed);pm=statistics.mean(predicted);ov=sum((v-om)**2 for v in observed);pv=sum((v-pm)**2 for v in predicted)
 return sum((a-om)*(b-pm) for a,b in zip(observed,predicted))/math.sqrt(ov*pv) if ov*pv else 0
rows=[]
for i,profiles in enumerate(all_profiles):
 side_rows={}
 for side in ('left','right'):
  candidates=[{'downward_phase_pixels':p*.25,'correlation':score(profiles[side],templates[side],p*.25)} for p in range(28)];candidates.sort(key=lambda r:-r['correlation']);side_rows[side]={'best':candidates[0],'candidates':candidates}
 rows.append({'frame':i,'source_sha256':hashlib.sha256((SOURCE/frames[i]['image']).read_bytes()).hexdigest(),**side_rows})
OUT.write_text(json.dumps({'status':'Source-only diagnostic; native7pixel periodic alpha width, descending-disc row exclusion; raster bias and identity ambiguity remain','templates':templates,'rows':rows},indent=2)+'\n');print([(r['frame'],r['left']['best'],r['right']['best']) for r in rows])
