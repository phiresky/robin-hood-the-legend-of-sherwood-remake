"""Revise coarse fence-boundary labels from close native shape/context evidence."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def main():
    directory=OUT/'missing-fence-candidates/boundary-roles95-v1';directory.mkdir(exist_ok=False)
    original=OUT/'understory-candidates/mixed75-91-boundary-review/75-proposed-fence95.png';mask=np.asarray(Image.open(original).convert('L'))>0;yy,xx=np.indices(mask.shape)
    cap=mask&(xx>=1558)&(xx<=1562)&(yy<=725)
    retained=mask&(xx==1604)&(yy==706)
    foliage=mask&(xx>=1609)
    ground=mask&~(cap|retained|foliage)
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA');crop=(1549,699,1635,738);overlay=np.asarray(source.convert('RGB')).copy();records=[]
    for name,array,color,reason in [('cap95',cap,(255,0,255),'Small post-top continuation above original y726 cap; closed additive candidate prepared.'),('existing95',retained,(255,100,0),'Gray native post-cap boundary already adjacent to existing fourth post; inferred wood edge, no widened gap plate.'),('foliage75',foliage,(0,255,100),'Green patches above/below the upper rail follow adjacent shrub/background growth, outside coherent timber stripe. Close context, not color alone, drives this proposal.'),('ground',ground,(255,255,0),'Tan/dark litter and shadow left of the visible post contours; inferred background receiver rather than widened timber.')]:
        path=directory/f'{name}.png';Image.fromarray(array.astype('uint8')*255).save(path);overlay[array]=color
        cut=source.copy();cut.putalpha(Image.fromarray(array.astype('uint8')*255));bg=Image.new('RGBA',source.size,(80,80,80,255));bg.alpha_composite(cut);bg.crop(crop).resize((860,390),Image.Resampling.NEAREST).save(directory/f'{name}-source.png')
        records.append(dict(role=name,pixels=int(array.sum()),mask=str(path),mask_sha256=sha(path),reason=reason,certainty='inferred; root review pending',receiving_geometry='new cap candidate' if name=='cap95' else 'coverage review required'))
    if not np.array_equal(cap|retained|foliage|ground,mask):raise ValueError('Incomplete boundary split')
    Image.fromarray(overlay).crop(crop).resize((860,390),Image.Resampling.NEAREST).save(directory/'overlay.png')
    write_json(directory/'proposal.json',dict(status='private correction to prior inferred labels; no canonical mutation',original_proposal=str(original),original_proposal_sha256=sha(original),prior_accepted_receipt_sha256=sha(OUT/'understory-candidates/mixed75-91-source-v2/boundary-review.json'),source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),records=records,reason='Native close grid shows prior51-pixel all-fence label crosses green gaps and tan litter. Geometry must follow observed wood rather than satisfy an incorrect receiver label.'))
    print([(r['role'],r['pixels']) for r in records])

if __name__=='__main__':main()
