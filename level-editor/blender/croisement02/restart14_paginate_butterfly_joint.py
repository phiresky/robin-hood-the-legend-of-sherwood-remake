"""Paginate an existing butterfly analytical sheet without resampling its pixels."""
from pathlib import Path
import hashlib,json
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies/joint99-cpu-v2';OUT=BASE/'pagination-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);source=BASE/'all99-baseline-joint-proposal.png';before=sha(source);original=Image.open(source).convert('RGB');assert original.size==(2640,1188);records=[]
 for page in range(11):
  image=Image.new('RGB',(720,442),'#252525');draw=ImageDraw.Draw(image);draw.text((8,5),f'Butterfly01 phases{page*9:02d}-{page*9+8:02d}: each pair previous(left) / joint proposal(right)',fill='white');draw.text((8,22),'Original source art under outlines. No resampling; coverage labels are analytical, not rendered.',fill='white');tiles=[]
  for index in range(9):
   phase=page*9+index;box=(phase%11*240,phase//11*132,phase%11*240+240,phase//11*132+132);tile=original.crop(box);at=(index%3*240,46+index//3*132);image.paste(tile,at);assert np.array_equal(np.array(tile),np.array(image.crop((at[0],at[1],at[0]+240,at[1]+132))));tiles.append({'phase':phase,'source_box':box,'page_top_left':at,'pixel_exact':True})
  path=OUT/f'page-{page:02d}.png';image.save(path);records.append({'file':path.name,'sha256':sha(path),'tiles':tiles})
 assert sha(source)==before;report={'source_sheet':str(source),'source_sha256':before,'original_dimensions':original.size,'phase_count':99,'copied_tiles_pixel_exact':True,'resampling':False,'source_unchanged':True,'pages':records};(OUT/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print('PAGINATED',len(records),'pages99exacttiles')
if __name__=='__main__':main()
