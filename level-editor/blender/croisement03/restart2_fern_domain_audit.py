"""Check corrected fern domains without treating excluded trunk pixels as leaves."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    for k in [35,76]:
        w=OUT/f'restart2/fern-ownership-v1/assets/croisement03-fern-{k}';r=json.loads((w/'inspection/source-comparison/report.json').read_text());x0,y0,x1,y1=r['crop']
        assert hashlib.sha256((w/'model.blend').read_bytes()).hexdigest()==r['model_sha256']
        hit=np.asarray(Image.open(w/'inspection/source-comparison/render.png').convert('RGBA'))[:,:,3]>127
        native=Image.new('L',(1408,960));native.paste(Image.open(OUT/f'baseline/masks/{k:06}.png'),level['masks'][k]['box_top_left']);n=np.asarray(native)[y0:y1,x0:x1]>0
        excluded=np.asarray(Image.open(OUT/f'restart2/fern-wood-proposal-v1/{k}-proposed-wood.png'))[y0:y1,x0:x1]>0;owned=n&~excluded;missing=owned&~hit
        if missing.any():raise ValueError(f'Fern {k}: unexplained plant holes')
        old=np.asarray(Image.open(OUT/f'fern-candidates-v6/fern-{k:02}/observed-source.png').convert('RGBA'));new=np.asarray(Image.open(OUT/f'restart2/fern-ownership-v1/fern-{k:02}/observed-source.png').convert('RGBA'));known=new[:,:,3]>127
        if not np.array_equal(old[known],new[known]):raise ValueError(f'Fern {k}: retained known RGBA changed')
        report=dict(model_sha256=r['model_sha256'],native_mask=k,native_mask_pixels=int(n.sum()),wood_exclusion_pixels=int(excluded.sum()),owned_plant_pixels=int(owned.sum()),unexplained_missing_pixels=int(missing.sum()),retained_known_rgba_exact=True,scope='Native source plant domain excludes only traced bark pixels. Other material-ownership ambiguities and coarse wood integration are not resolved by coverage alone.')
        (w/'inspection/owned-source-coverage.json').write_text(json.dumps(report,indent=2)+'\n');print(k,report['owned_plant_pixels'])
if __name__=='__main__':main()
