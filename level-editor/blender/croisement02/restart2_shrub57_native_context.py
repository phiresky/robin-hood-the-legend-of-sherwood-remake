"""Explain the shrub's sparse native silhouette with its exact source ownership."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def main(version=9):
 base=OUT/f'restart2-fence/shrub57-sign-bend-v{version}';dest=base/'native-context';dest.mkdir(exist_ok=False)
 report=json.loads((base/'report.json').read_text());assert sha(base/'model.blend')==report['model_sha256']
 original=OUT/'understory-round-9/assets/croisement02-shrub-57/inspection/source-coverage/report.json';coverage=json.loads(original.read_text());mask=Path(coverage['coverage_domain']['png']);assert sha(mask)==coverage['coverage_domain']['png_sha256']
 source=OUT/'animation-references/composite-frame-0.png';box=(-58,188,134,380);art=Image.open(source).convert('RGBA').crop(box);domain=np.array(Image.open(mask).convert('L').crop(box))>127
 highlighted=np.array(art);highlighted[~domain,:3]=(highlighted[~domain,:3].astype(float)*.3).astype(np.uint8)
 panels=[art.resize((384,384),Image.Resampling.NEAREST),Image.fromarray(highlighted).resize((384,384),Image.Resampling.NEAREST),Image.open(base/'native-after.png').convert('RGBA')];sheet=Image.new('RGB',(1152,416),(65,65,65))
 for i,(im,label) in enumerate(zip(panels,['Native game art','Assigned foliage highlighted in context','Saved shrub: native camera'])):
  sheet.paste(im,(i*384,0),im.getchannel('A'));ImageDraw.Draw(sheet).text((i*384+4,390),label,fill='white')
 sheet.save(dest/'source-context.png')
 write_json(dest/'report.json',dict(status='Bound source-context explanation; no new rendering or source assignment',model_sha256=report['model_sha256'],source_sha256=sha(source),domain_sha256=sha(mask),source_crop=list(box),domain_index=481,source_reference_report_sha256=sha(original),native_saved_render_sha256=sha(base/'native-after.png'),image_sha256=sha(dest/'source-context.png'),notes=['The visible foliage follows the upper margins of native rock/bank artwork; its isolated native silhouette is intentionally sparse.','The rounded rear volume is inferred. Source-ray depth changes preserve original saved first-hitRGBA exactly.','The original approved shrub source report retains9 missing and12 extra contour pixels; exact saved-RGBA preservation does not claim100% native domain coverage.','Out-of-map context remains unknown; gray outside the source is not observed ground.']))
 print(dest)
if __name__=='__main__':main()
