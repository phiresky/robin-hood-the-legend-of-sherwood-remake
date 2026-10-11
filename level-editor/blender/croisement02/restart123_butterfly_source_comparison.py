"""Compare saved source-camera images without applying registration twice."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--review',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads(a.fit.read_text());review=json.loads((a.review/'report.json').read_text());assert sha(a.fit)==review['fit_sha256'];scale=24*fit['fixed_uniform_scale'];images=[]
 for first in range(0,99,25):
  sheet=Image.new('RGB',(1280,760),'#252525');draw=ImageDraw.Draw(sheet)
  for offset,row in enumerate(fit['rows'][first:first+25]):
   sourcep=Path(row['source']['source']);assert sha(sourcep)==row['source']['sha256'];arr=np.asarray(Image.open(sourcep).convert('RGBA'));yy,xx=np.nonzero(arr[:,:,3]>0);center=np.array([xx.mean()+.5,yy.mean()+.5]);gy,gx=np.mgrid[:128,:128];sx=np.floor((gx+.5-64)*scale/128+center[0]).astype(int);sy=np.floor((gy+.5-64)*scale/128+center[1]).astype(int);valid=(sx>=0)&(sy>=0)&(sx<arr.shape[1])&(sy<arr.shape[0]);pixels=np.zeros((128,128,4),np.uint8);pixels[valid]=arr[sy[valid],sx[valid]];source=Image.new('RGBA',(128,128),'#252525');source.alpha_composite(Image.fromarray(pixels));render=Image.open(a.review/f"anatomy/{row['phase']:03}.png").convert('RGBA').resize((128,128),Image.Resampling.NEAREST)
   # Saved review camera follows the source anchor; exported translation already includes registration.
   model=Image.new('RGBA',(128,128),'#252525');model.alpha_composite(render);x=offset%5*256;y=offset//5*152;sheet.paste(source.convert('RGB'),(x,y+20));sheet.paste(model.convert('RGB'),(x+128,y+20));draw.text((x+3,y+3),f"{row['phase']:02} source | saved model",fill='white')
  dest=a.output/f'comparison-{first:02}.png';sheet.save(dest);images.append(dict(path=str(dest),sha256=sha(dest)))
 (a.output/'report.json').write_text(json.dumps(dict(status='PRIVATE_SOURCE_COMPARISON_NOT_APPROVAL',fit_sha256=sha(a.fit),review_sha256=sha(a.review/'report.json'),recipe_sha256=sha(Path(__file__)),images=images,registration='Applied once by model translation. Camera follows source anchor, so no additional image translation.',limits=['Native sprite and physical rendered silhouette are separate hypotheses; this comparison does not waive residuals or prove flight contacts.']),indent=2)+'\n')
if __name__=='__main__':main()
