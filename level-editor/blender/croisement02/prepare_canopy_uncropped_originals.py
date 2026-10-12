"""Supplement native-cropped phase evidence with full original-direction views."""
from pathlib import Path
import json,hashlib,subprocess,shutil
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';S=B/'canopy06-complete-calibrated-playerfix-oct12-v1';O=B/'canopy-full-original-supplement-oct12-v1';O.mkdir(exist_ok=False)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
rows=[]
for group in [2,3,5,6,7]:rows.extend(json.loads((B/f'canopy{group:02}-complete-calibrated-playerfix-oct12-v1/request.json').read_text())['rows'])
(O/'request.json').write_text(json.dumps({'rows':rows},indent=2)+'\n')
for name in ['run.mjs','build.mjs']:(O/name).write_text((S/name).read_text().replace(S.name,O.name))
s=(S/'proof.ts').read_text();start=s.index('await nativeView(tick,object);if(tick===0');end=s.index('}});if(current)',start);s=s[:start]+"if(tick===28){renderer.setSize(768,768);await capture(tick,object,0)};"+s[end:];s=s.replace('Complete group review:15exact native-bbox phase views,7additional directions,isolated actual junctions at0/7,transparent at0/7,and cutaway at7.','Supplement only: full uncropped original direction at phase7 for each group; existing calibrated phase and otherdirection evidence retained.')
(O/'proof.ts').write_text(s);subprocess.run(['node',str(O/'build.mjs')],check=True);subprocess.run(['node','--check',str(O/'run.mjs')],check=True)
pins=json.loads((O/'source-pins.json').read_text())
for path,digest in list(pins.items()):
 p=Path(path);assert sha(p)==digest;dest=O/'runtime-snapshot'/(p.relative_to(R)if p.is_absolute()else p);dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dest);pins[str(dest)]=digest

def collect(v):
 if isinstance(v,dict):
  if 'path'in v and 'sha256'in v:
   p=Path(v['path']);assert sha(p)==v['sha256'];pins[str(p)]=v['sha256']
  for x in v.values():collect(x)
 elif isinstance(v,list):
  for x in v:collect(x)
collect(rows)
for name in ['request.json','run.mjs','build.mjs','proof.ts','proof.js','index.html']:pins[str(O/name)]=sha(O/name)
(O/'source-pins.json').write_text(json.dumps(pins,indent=2)+'\n');(O/'handoff.json').write_text(json.dumps({'status':'PREPARED_NOT_LAUNCHED','groups':[2,3,5,6,7],'captures':5,'scope':'Full uncropped original direction at tick28 only; original calibrated phasepairs and existing seven other directions retained. Same candidate models and production113tick checks.','runner':{'path':str(O/'run.mjs'),'sha256':sha(O/'run.mjs')},'pins':{'path':str(O/'source-pins.json'),'sha256':sha(O/'source-pins.json')}},indent=2)+'\n');print(sha(O/'handoff.json'))
