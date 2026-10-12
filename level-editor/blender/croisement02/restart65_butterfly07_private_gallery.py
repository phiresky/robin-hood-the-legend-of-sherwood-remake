"""Freeze a source-aligned private review of observed-texture butterfly articulation."""
import json,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02'
MODEL=BASE/'restart65-butterfly07-exact-model-review-v1';FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json';OUT=BASE/'restart65-butterfly07-private-gallery-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());pins=[]
 for first in range(0,99,25):
  sheet=Image.new('RGB',(1280,720),'#252525');draw=ImageDraw.Draw(sheet)
  for offset,row in enumerate(fit['rows'][first:first+25]):
   arr=np.array(Image.open(row['source']['source']).convert('RGBA'));yy,xx=np.nonzero(arr[:,:,3]>0);center=np.array([xx.mean()+.5,yy.mean()+.5]);gy,gx=np.mgrid[:128,:128];sx=np.floor((gx+.5-64)*24/128+center[0]).astype(int);sy=np.floor((gy+.5-64)*24/128+center[1]).astype(int);valid=(sx>=0)&(sy>=0)&(sx<arr.shape[1])&(sy<arr.shape[0]);a=np.zeros((128,128,4),np.uint8);a[valid]=arr[sy[valid],sx[valid]];source=Image.new('RGBA',(128,128),'#252525');source.alpha_composite(Image.fromarray(a));model=Image.open(MODEL/'native-motion'/f"{row['phase']:03}.png").convert('RGBA');render=Image.new('RGBA',(128,128),'#252525');render.alpha_composite(model)
   x=offset%5*256;y=offset//5*144;sheet.paste(source.convert('RGB'),(x,y+16));sheet.paste(render.convert('RGB'),(x+128,y+16));draw.text((x,y),f"{row['phase']} source | saved model",fill='white')
  sheet.save(OUT/f'comparison-{first:02}.png')
 for src,name in [(MODEL/'actual/sheet.png','actual.png'),(MODEL/'solid/sheet.png','solid.png'),(MODEL/'native-motion.gif','motion.gif')]:shutil.copyfile(src,OUT/name)
 for p in [MODEL/'model.blend',MODEL/'report.json',FIT,*sorted(OUT.glob('*.png')),OUT/'motion.gif']:pins.append(dict(path=str(p),sha256=sha(p)))
 report=dict(status='PRIVATE_REVIEW_HOLD_NOT_APPROVAL_READY',pins=pins,source_fit=dict(source_pixels=5015,missing_centers=250,bright_missing_centers=54,extra_centers=578),source_clock=dict(phases=99,cycle_ticks=198),self_review=['Original camera is top-left in both eight-view sheets.','Exact source RGB selection restores yellow and blue contrast lost in the discarded averaged-atlas trial.','Gray body and small wing texels remain unassigned, requiring separate appearance scope.','Some wide source poses remain mismatched; do not describe this as pixel-exact reproduction.','99 discrete poses reviewed; native timing retained. Continuous flight-path and render interpolation are not certified.'],pending=['Fresh conservative contact audit for this exact fit.','Source silhouette residual decision/refinement before grouped user approval.','Body source ownership and final texture scope.','Full flight-path exported animation and browser integration.'],synthesis_requested=False,canonical_writes=False)
 (OUT/'handoff.json').write_text(json.dumps(report,indent=2)+'\n')
 (OUT/'index.html').write_text('''<!doctype html><meta charset="utf-8"><title>Butterfly07 private review</title><style>body{background:#202020;color:#ddd;font:16px sans-serif;margin:28px}img{max-width:100%;display:block;margin:16px 0}p{max-width:1000px}</style><h1>Butterfly07 — private geometry and motion review</h1><p>Not approval-ready. One fixed anatomy across 99 poses, 198 ticks. Top-left is the original art camera. Wing colors copy exact observations from this butterfly only; gray body and small wing regions remain unknown. No synthesis or publication.</p><p>Bounded pose corrections reduce missed source centers 344→250 and bright misses70→54. Some wide poses remain mismatched. These are 2D-constrained hypotheses; hidden depth and anatomy are inferred.</p><h2>Eight saved-model views</h2><img src="actual.png"><img src="solid.png"><h2>Discrete native-timed local motion</h2><img src="motion.gif"><p>Flight path not shown. Intermediate articulation/continuous receiver clearance requires separate proof.</p><h2>Original artwork versus saved model, aligned to the same source camera scale</h2>'''+''.join(f'<img src="comparison-{first:02}.png">' for first in range(0,99,25)))
 print(str(OUT/'handoff.json'))
if __name__=='__main__':main()
