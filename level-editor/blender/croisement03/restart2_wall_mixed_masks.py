"""Expose mixed wall/foliage mask scope without changing material ownership."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageChops
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    source=OUT/'baseline/covered.png';art=Image.open(source).convert('RGBA');level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());destination=OUT/'restart2/southeast-wall-source';destination.mkdir(parents=True,exist_ok=True)
    def mask(number):
        image=Image.new('L',art.size);image.paste(Image.open(OUT/f'baseline/masks/{number:06}.png'),tuple(level['masks'][number]['box_top_left']));return image
    wall=mask(113);rows=[]
    for number in [25,69,70]:rows.append(dict(native_mask=number,overlap_with_wall113=int(np.count_nonzero((np.array(wall)>127)&(np.array(mask(number))>127)))))
    counterfactual=ImageChops.subtract(ImageChops.subtract(wall,mask(69)),mask(70));image=Image.new('RGBA',art.size,'#444');image.paste(art,(0,0),counterfactual);image.crop((1138,746,1408,841)).convert('RGB').resize((1080,380),Image.Resampling.NEAREST).save(destination/'wall-after-leaf-exclusions.png')
    row=level['masks'][69];x,y=row['box_top_left'];w,h=row['box_size'];image=Image.new('RGBA',art.size,'#444');image.paste(art,(0,0),mask(69));image.crop((x,y,x+w,y+h)).convert('RGB').resize((w*4,h*4),Image.Resampling.NEAREST).save(destination/'native69-source.png')
    (destination/'mixed-mask-audit.json').write_text(json.dumps(dict(source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),overlaps=rows,status='Diagnostic only; material ownership unchanged',finding='Native69 contains both foreground leaves and the prominent pink stone cap. Blanket subtraction from wall113 removes genuine masonry. Trace material boundaries and receiving geometry before assigning those source pixels.',worker_unchanged='restart2/stone-wall-v1/assets/croisement03-southeast-stone-wall/model.blend'),indent=2)+'\n')
if __name__=='__main__':main()
