"""Correct supplementary firewood ownership in a fresh texture experiment."""
import json,sys,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageChops
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
OUT=ROOT/'level-editor/work/croisement03-refinement';R=OUT/'restart2';ASSET='croisement03-southwest-firewood-stack'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    parent=R/'texture-round1'/ASSET;old=parent/'experiment';destination=parent/'experiment-reference-retry1'
    review=json.loads((old/'prebake-review.json').read_text());review.update(status='HOLD-reference-ownership',reason='Supplementary own-native crop included23 foreground foliage45 pixels. Approved target input/protected pixels remain correct. Raw and preserved outputs are retained; corrected reference retry replaces this candidate.');(old/'prebake-review.json').write_text(json.dumps(review,indent=2)+'\n')
    prepare(parent/'review-manifest.json',ASSET,destination,parent/'decisions.json');refs=destination/'material-references';refs.mkdir();entries=[]
    old_manifest=json.loads((old/'auxiliary-references.json').read_text());donor=old_manifest['references'][0];copied=refs/Path(donor['file']).name;shutil.copyfile(donor['file'],copied);donor['file']=str(copied);assert sha(copied)==donor['sha256'];entries.append(donor)
    source=OUT/'baseline/covered.png';image=Image.open(source).convert('RGBA');domain=Image.open(R/'firewood-v7/observed-wood-domain.png').convert('L');before=np.array(domain)>127;level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    for number in [45,54]:
        excluded=Image.new('L',domain.size);excluded.paste(Image.open(OUT/f'baseline/masks/{number:06}.png'),tuple(level['masks'][number]['box_top_left']));domain=ImageChops.subtract(domain,excluded)
    assert int(before.sum()-(np.array(domain)>127).sum())==23
    domain.save(refs/'authoritative-wood-only-domain.png');image.putalpha(domain);image=image.crop((419,758,457,788)).resize((228,180),Image.Resampling.NEAREST);pixels=np.array(image);pixels[pixels[:,:,3]==0,:3]=0;target=refs/'croisement03-observed-firewood.png';Image.fromarray(pixels).save(target)
    entries.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=ASSET,role='Only positively traced native wood after subtracting foreground foliage45/54. Continue warm brown bark and pale woody end grain, without leaf fragments or copying the example layout.',parent_sha256=sha(source),ownership_sha256=sha(refs/'authoritative-wood-only-domain.png')))
    (destination/'auxiliary-references.json').write_text(json.dumps(dict(version=1,input_sha256=sha(destination/'input.png'),lighting_sha256=sha(destination/'solid.png'),references=entries),indent=2)+'\n')
    (destination/'retry-provenance.json').write_text(json.dumps(dict(parent=str(old),reason=review['reason'],removed_foreground_reference_pixels=23,approved_geometry_changed=False,approved_input_changed=sha(old/'input.png')!=sha(destination/'input.png')),indent=2)+'\n');print(destination)
if __name__=='__main__':main()
