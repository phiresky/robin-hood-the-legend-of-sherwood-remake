"""Bind source RGBA and adjacent-bank guards to a saved private root review."""
import sys,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'


def main(variant):
 base=OUT/'restart3-tree06-root'/variant
 audit=OUT/'restart3-northern-source-audit/report.json'
 region=next(r for r in json.loads(audit.read_text())['regions']if r['region']==8)
 paths=[OUT/'baseline/covered.png',base/'model.blend',base/'review/native-bank-before.png',base/'review/native-bank-after.png',audit]
 source=np.array(Image.open(paths[0]).convert('RGBA'));before=np.array(Image.open(paths[2]).convert('RGBA'));after=np.array(Image.open(paths[3]).convert('RGBA'))
 wood=[];bank=[]
 for row in region['pixels']:
  x,y=row['pixel'];px=int((x+.5-594)*384/112);py=int((y+.5-464)*384/112)
  if row['classification']=='wood_domain_residual':
   wood.append(dict(pixel=[x,y],maximum_rgba_error=int(np.abs(source[y,x].astype(int)-after[py,px].astype(int)).max())))
  else:
   bank.append(dict(pixel=[x,y],maximum_rgba_change=int(np.abs(before[py,px].astype(int)-after[py,px].astype(int)).max())))
 exact=sum(r['maximum_rgba_error']==0 for r in wood);fixed=sum(r['maximum_rgba_change']==0 for r in bank)
 mask=np.asarray(Image.open(OUT/'baseline/masks/000006.png').convert('L'))>0
 context=dict(exact_wood_before=0,exact_wood_after=0,previous_exact_wood_regressions=0,outside_wood_changed_samples=0,outside_wood_maximum_error=0)
 for y in range(464,576):
  for x in range(594,706):
   px=int((x+.5-594)*384/112);py=int((y+.5-464)*384/112)
   own=0<=y-252<mask.shape[0] and 0<=x-517<mask.shape[1] and mask[y-252,x-517]
   a=np.array_equal(before[py,px],source[y,x]);b=np.array_equal(after[py,px],source[y,x])
   error=int(np.abs(before[py,px].astype(int)-after[py,px].astype(int)).max())
   if own:
    context['exact_wood_before']+=int(a);context['exact_wood_after']+=int(b);context['previous_exact_wood_regressions']+=int(a and not b)
   elif error:
    context['outside_wood_changed_samples']+=1;context['outside_wood_maximum_error']=max(context['outside_wood_maximum_error'],error)
 report=dict(status='PASS'if exact==313 and fixed==131 else 'HOLD',sources={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths},sampling='384px square render over112 native pixels; nearest rendered sample to each native pixel center; position difference<0.146 native pixels per axis. Nearest source texture filtering.',wood_pixels=wood,bank_pixels=bank,exact_target_rgba=exact,unchanged_bank_samples=fixed,limitations=['Scoped native wood313 and bank131 appearance guards only; not a whole-scene collision or appearance parity claim.','Root depth and buried connection are inferred; independent all-angle review remains required.'])
 report['neighborhood']=context
 if context['previous_exact_wood_regressions'] or context['outside_wood_maximum_error']>1:report['status']='HOLD'
 (base/'native-target-verification.json').write_text(json.dumps(report,indent=2)+'\n')
 print(dict(status=report['status'],exact_target_rgba=exact,unchanged_bank_samples=fixed))
 assert report['status']=='PASS'


if __name__=='__main__':main(sys.argv[1])
