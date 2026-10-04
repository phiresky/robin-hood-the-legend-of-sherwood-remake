"""Prepare explicitly provisional temporal ownership using only native phase RGBA."""
import json,hashlib
from pathlib import Path
import numpy as np
from scipy.ndimage import binary_dilation
from PIL import Image
from catalog import OUT,tree_workspace

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 dest=OUT/'tree41-phase-appearance-proof-v2';dest.mkdir(exist_ok=False)
 worker=tree_workspace(41);base=worker/'inspection/irregular-crown-edge/complete-source.png';rgba=np.array(Image.open(base).convert('RGBA'));known=rgba[:,:,3]>127
 animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];native=animations[3]
 packet=json.loads((OUT/'forest-v4-sources/tree-41/partition.json').read_text());x,y,w,h=packet['native_bbox'];assert rgba.shape==(h,w,4)
 domain=binary_dilation(known,iterations=5);phases=[];arrays=[]
 for f in native['frames']:
  image=Image.new('RGBA',(w,h));image.alpha_composite(Image.open(f['image']).convert('RGBA'),(f['bbox'][0]-x,f['bbox'][1]-y));arrays.append(np.array(image))
 initial_overlay=arrays[0][:,:,3]>127
 temporal=np.logical_or.reduce([(a[:,:,3]>127)&~initial_overlay&domain for a in arrays])|known
 Image.fromarray((temporal*255).astype(np.uint8)).save(dest/'temporal-domain.png');visual=np.zeros((h,w,3),np.uint8);visual[known]=[60,110,60];visual[temporal&~known]=[255,0,255];Image.fromarray(visual).save(dest/'temporal-union.png')
 for index,(f,source) in enumerate(zip(native['frames'],arrays)):
  result=rgba.copy()
  if index:
   # Transparent animation pixels reveal the painted static canopy, not empty space.
   composite=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop((x,y,x+w,y+h))
   for animation in animations:
    frame=animation['frames'][index if animation['index']==3 else 0]
    composite.alpha_composite(Image.open(frame['image']).convert('RGBA'),(frame['bbox'][0]-x,frame['bbox'][1]-y))
   alpha=known|((source[:,:,3]>127)&~initial_overlay&domain);result[temporal,3]=0;result[alpha,:3]=np.array(composite)[alpha,:3];result[alpha,3]=255
  target=dest/f'phase-{index:02}.png'
  if not index:target.write_bytes(base.read_bytes())
  else:Image.fromarray(result).save(target)
  assert np.array_equal(result[~temporal],rgba[~temporal])
  phases.append(dict(index=index,image=str(target),sha256=sha(target),source=f['image'],source_sha256=sha(Path(f['image'])),delay=f['delay'],opaque_pixels=int((result[:,:,3]>127).sum()),source_bbox=f['bbox']))
 (dest/'inputs.json').write_text(json.dumps(dict(model=str(worker/'model.blend'),model_sha256=sha(worker/'model.blend'),covered_source_sha256=sha(OUT/'baseline/covered.png'),frozen_context_animation_frames=[dict(index=a['index'],image=a['frames'][0]['image'],sha256=sha(Path(a['frames'][0]['image']))) for a in animations if a['index']!=3],temporal_domain_sha256=sha(dest/'temporal-domain.png'),approved_atlas=str(base),approved_atlas_sha256=sha(base),atlas_bbox=[x,y,w,h],phases=phases,temporal_new_pixels=int((temporal&~known).sum()),outside_temporal_domain_unchanged=True,phase0_byte_identical=True,ownership='Keep approved static canopy alpha. Add only native phase overlay pixels absent from overlay phase0 within5px of approved alpha. RGB is static map+native overlays in frozen order, varying only own sequence3; neighboring-crown temporal ownership requires joint review.',status='new temporal appearance candidate, not approved'),indent=2)+'\n')
 print(dest)
if __name__=='__main__':main()
