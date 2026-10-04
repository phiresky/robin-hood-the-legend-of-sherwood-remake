"""Separate conservative visible painted-ground shadow pixels from native sign poses."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_dilation
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from fit_native_sign import OUT,raster
from evidence_io import sha,write_json

def main():
 base=OUT/'state-sign-candidate';fit=json.loads((base/'fit-v6/fit.json').read_text());p=list(fit['parameters'].values());dst=base/'painted-shadow-v2';dst.mkdir(exist_ok=False);frames=next(r for r in fit['profile']['rows']if r['action_id']==0)['frames'];records=[];sheet=Image.new('RGB',(1536,960),(80,80,80));yy,xx=np.indices((72,64))
 for i,f in enumerate(frames):
  source=Image.open(f['image']).convert('RGBA');canvas=Image.new('RGBA',(64,72));canvas.paste(source,(32+int(f['offset'][0]),55+int(f['offset'][1])));rgba=np.asarray(canvas);black=(rgba[:,:,:3]==0).all(2)&(rgba[:,:,3]>127);body=raster(p,i);shadow=black&(((yy>42)&~binary_dilation(body,iterations=2))|(yy>=54));output=np.zeros_like(rgba);output[shadow]=[0,0,0,255];path=dst/f'shadow-{i:02}.png';Image.fromarray(output).save(path);annotated=np.full((72,64,3),80,np.uint8);visible=rgba[:,:,3]>127;annotated[visible]=rgba[visible,:3];annotated[shadow]=[0,255,150];sheet.paste(Image.fromarray(annotated).resize((192,216),Image.Resampling.NEAREST),((i%8)*192,(i//8)*240));ImageDraw.Draw(sheet).text(((i%8)*192+3,(i//8)*240+221),f'{i}: {int(shadow.sum())} px',fill='white');records.append(dict(frame=i,source=f['image'],source_sha256=f['image_sha256'],painted_ground_pixels=int(shadow.sum()),black_pixels_not_classified_as_ground=int((black&~shadow).sum()),pure_blue_opaque_pixels=int(((rgba[:,:,:3]==[0,0,255]).all(2)&visible).sum()),mask=str(path),mask_sha256=sha(path)))
 sheet.save(dst/'source-segmentation.png');write_json(dst/'proposal.json',dict(status='Conservative source-ground proposal; independent review and visible pose validation pending',records=records,rule='Only exact black opaque pixels near ground (source_y>-13) and outside a2pixel dilation of the fitted physical body, plus exact native black at source_y>=-1 as inferred foot-contact paint. Black is not treated as a reserved shadow key.',coordinates=dict(canvas_size=[64,72],anchor=[32,55]),provenance='These pixels are native source RGB; their assignment to a ground receiver is inferred from context. Upper ambiguous body black stays unclassified; near-foot source black is explicitly inferred contact paint.',limitations=['Conservative subset, not full shadow completeness.','No new black pixels invented; all original32RGBA frames remain authoritative.','Native source and oblique ground rendering still require inspection.']))
 print([(r['frame'],r['painted_ground_pixels'])for r in records])

if __name__=='__main__':main()
