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
 report=dict(status='PASS'if exact==313 and fixed==131 else 'HOLD',sources={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths},sampling='384px square render over112 native pixels; nearest rendered sample to each native pixel center; position difference<0.146 native pixels per axis. Nearest source texture filtering.',wood_pixels=wood,bank_pixels=bank,exact_target_rgba=exact,unchanged_bank_samples=fixed,limitations=['Scoped native wood313 and bank131 appearance guards only; not a whole-scene collision or appearance parity claim.','Root depth and buried connection are inferred; independent all-angle review remains required.'])
 (base/'native-target-verification.json').write_text(json.dumps(report,indent=2)+'\n')
 print(dict(status=report['status'],exact_target_rgba=exact,unchanged_bank_samples=fixed))
 assert report['status']=='PASS'


if __name__=='__main__':main(sys.argv[1])
