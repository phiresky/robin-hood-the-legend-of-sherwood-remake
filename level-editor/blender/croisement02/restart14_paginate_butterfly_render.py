"""Copy saved-model source/native/rear review tiles into exact compact pages."""
from pathlib import Path
import hashlib,json
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies/rig-joint-trial-v1';OUT=BASE/'pagination-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);source=BASE/'all99-source-native-rear.png';before=sha(source);image=Image.open(source).convert('RGB');assert image.size==(3168,1188);pages=[]
 for page in range(11):
  result=Image.new('RGB',(864,442),'#252525');draw=ImageDraw.Draw(result);draw.text((8,5),f'Butterfly01 phases{page*9:02d}-{page*9+8:02d}: source / saved native / saved rear at same scale',fill='white');draw.text((8,22),'Frozen anatomy and fixed pattern. Original source retained separately; rendered parity is not claimed.',fill='white');tiles=[]
  for item in range(9):
   phase=page*9+item;box=(phase%11*288,phase//11*132,phase%11*288+288,phase//11*132+132);tile=image.crop(box);at=(item%3*288,46+item//3*132);result.paste(tile,at);assert np.array_equal(np.array(tile),np.array(result.crop((at[0],at[1],at[0]+288,at[1]+132))));tiles.append({'phase':phase,'source_box':box,'page_top_left':at,'pixels_exact':True})
  file=OUT/f'page-{page:02d}.png';result.save(file);pages.append({'file':file.name,'sha256':sha(file),'tiles':tiles})
 assert before==sha(source);report={'model_sha256':sha(BASE/'model.blend'),'validation_sha256':sha(BASE/'validation.json'),'source_sheet_sha256':before,'all99_tiles_pixel_exact':True,'resampling':False,'pages':pages};(OUT/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
