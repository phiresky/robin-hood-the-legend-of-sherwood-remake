"""Bind observed supplementary material examples to approved Croisement03 fills."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageChops
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement';R=OUT/'restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,j):p.write_text(json.dumps(j,indent=2)+'\n')
def clean(image):
    pixels=np.array(image.convert('RGBA'));pixels[pixels[:,:,3]==0,:3]=0;return Image.fromarray(pixels)
def main():
    for asset in ['croisement03-fern-35','croisement03-fern-76','croisement03-southwest-firewood-stack','croisement03-stream-fallen-log']:
        experiment=R/'texture-round1'/asset/'experiment';refs=experiment/'material-references';refs.mkdir(exist_ok=False);records=[]
        def add(source,donor,role,box=None,known=None,scale=3):
            image=Image.open(source).convert('RGBA');provenance=dict(parent_image=str(source),parent_sha256=sha(source),crop=box,integer_nearest_scale=scale)
            if known:
                mask=np.array(Image.open(known).convert('RGBA'))[:,:,0]>127;pixels=np.array(image);pixels[:,:,3]=np.where(mask,pixels[:,:,3],0);image=Image.fromarray(pixels);provenance.update(known_mask=str(known),known_mask_sha256=sha(known))
            if box:image=image.crop(tuple(box))
            image=image.resize((image.width*scale,image.height*scale),Image.Resampling.NEAREST);target=refs/(donor+'.png');clean(image).save(target);provenance['transparent_rgb_cleared']=True;write(target.with_suffix('.json'),provenance);records.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=donor,role=role,provenance=str(target.with_suffix('.json'))))
        if 'fern-' in asset:
            for number in [35,76]:
                add(R/f'fern-ownership-v1/fern-{number}/observed-source.png',f'croisement03-fern-{number}', 'Own native observed fern foliage after traced bark removal. Fine golden olive and brown frond texture only; preserve the target fixed leaves and openings, do not add a trunk or copy plant layout.')
        elif 'firewood' in asset:
            archive=ROOT/'level-editor/work/croisement02-refinement/user-reviews/croisement02-north-firewood-stack/da08898704c0518a20db03f6693188e832891e1c4a70bebe72bd156296df2dd8/modified/views'
            add(archive/'view-0-textured.png','croisement02-north-firewood-stack','Observed rough brown billet bark and pale cut wood ends; material grain example only. The target remains exactly three compact round logs, with its own lighting and proportions.',box=[130,110,168,148],known=archive/'view-0-known.png',scale=5)
            source=OUT/'baseline/covered.png';image=Image.open(source).convert('RGBA');domain=Image.open(R/'firewood-v7/observed-wood-domain.png').convert('L');level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
            for number in [45,54]:
                excluded=Image.new('L',domain.size);excluded.paste(Image.open(OUT/f'baseline/masks/{number:06}.png'),tuple(level['masks'][number]['box_top_left']));domain=ImageChops.subtract(domain,excluded)
            domain.save(refs/'authoritative-wood-only-domain.png');image.putalpha(domain);target=refs/'croisement03-observed-firewood.png';clean(image.crop((419,758,457,788)).resize((228,180),Image.Resampling.NEAREST)).save(target);records.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=asset,role='Target native timber and two observed pale end patches, isolated as material evidence only; do not copy or extend the source crop rectangle.',parent_sha256=sha(source),ownership_sha256=sha(refs/'authoritative-wood-only-domain.png')))
        else:
            source=OUT/'baseline/covered.png';image=Image.open(source).convert('RGBA');image.putalpha(Image.open(R/'fallen-log-v3/observed-domain.png').convert('L'));target=refs/'native-mossy-bark.png';clean(image.crop((637,871,721,910)).resize((336,156),Image.Resampling.NEAREST)).save(target);records.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=asset,role='Own observed mossy weathered bark at larger scale. Continue this material around hidden sides, following the target bent log; do not add foliage, rocks or a rectangular cutout.',parent_sha256=sha(source),ownership_sha256=sha(R/'fallen-log-v3/observed-domain.png')))
        write(experiment/'auxiliary-references.json',dict(version=1,input_sha256=sha(experiment/'input.png'),lighting_sha256=sha(experiment/'solid.png'),references=records))
if __name__=='__main__':main()
