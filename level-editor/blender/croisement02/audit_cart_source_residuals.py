"""Localize cart source residuals without turning uncertain actor/shadow pixels into solids."""
import hashlib, json, sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import label, find_objects
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'/(sys.argv[sys.argv.index('--candidate')+1] if '--candidate' in sys.argv else 'north-cart-initial-candidate-v3')

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    dest=BASE/'source-residual-audit';dest.mkdir(exist_ok=False)
    source=ROOT/'level-editor/work/croisement02-refinement/state-target-evidence/profiles/chariot03-10/action-160-direction-0-frame-000.png'
    rgba=np.array(Image.open(source).convert('RGBA'))
    domain=np.array(Image.open(BASE/'cart-source-domain.png'))>0
    actual=np.array(Image.open(BASE/'source-actual.png').convert('RGBA'))
    yy,xx=np.indices(domain.shape)
    # The source view is centered at native (150,77), with a 200-unit vertical span.
    rx=np.floor((xx+.5-50)*512/200).astype(int)
    ry=np.floor((yy+.5+23)*512/200).astype(int)
    inside=(rx>=0)&(rx<512)&(ry>=0)&(ry<512)
    covered=np.zeros(domain.shape,dtype=bool)
    covered[inside]=actual[ry[inside],rx[inside],3]>=128
    missing=domain&~covered
    components,n=label(missing)
    records=[]
    for i,box in enumerate(find_objects(components),1):
        if box is None:continue
        mask=components==i;count=int(mask.sum())
        if count<8:continue
        y,x=box
        records.append(dict(pixels=count,bbox=[x.start,y.start,x.stop,y.stop],mean_rgb=np.round(rgba[:,:,:3][mask].mean(axis=0),2).tolist()))
    records.sort(key=lambda x:-x['pixels'])
    Image.fromarray(missing.astype(np.uint8)*255).save(dest/'uncovered-native-domain.png')
    background=Image.new('RGBA',(rgba.shape[1],rgba.shape[0]),(40,40,40,255));background.alpha_composite(Image.fromarray(rgba))
    overlay=Image.new('RGBA',background.size);a=np.zeros_like(rgba);a[missing]=[255,0,180,150];overlay=Image.fromarray(a);highlight=background.copy();highlight.alpha_composite(overlay)
    canvas=Image.new('RGB',(background.width*6,background.height*3+28),(28,28,28));canvas.paste(background.resize((background.width*3,background.height*3),Image.Resampling.NEAREST),(0,28));canvas.paste(highlight.resize((background.width*3,background.height*3),Image.Resampling.NEAREST),(background.width*3,28));draw=ImageDraw.Draw(canvas);draw.text((8,8),'Native source / same source with uncovered manual domain',fill='white');canvas.save(dest/'source-residual-comparison.png')
    result=dict(status='localization only; manual source polygon is not proven semantic ownership',source_sha256=sha(source),model_sha256=sha(BASE/'worker.blend'),source_domain_sha256=sha(BASE/'cart-source-domain.png'),render_sha256=sha(BASE/'source-actual.png'),native_domain_pixels=int(domain.sum()),native_uncovered_pixels=int(missing.sum()),components=records,limits=['Dark native pixels beneath wheels and at harness boundary are unresolved source ownership, not automatically missing solid cart.','Exact blue shadow key is excluded from current manual domain; ordinary dark RGB is not automatically a shadow key.','Source residuals must be interpreted with actual multi-view geometry, not filled as a flat silhouette.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))
if __name__=='__main__':main()
