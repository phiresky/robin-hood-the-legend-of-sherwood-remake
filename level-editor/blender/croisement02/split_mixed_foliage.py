"""Partition mixed masks76/93 without creating duplicate fence or oak owners."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageFilter
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def main():
    output=OUT/'mixed-wood-audit';output.mkdir(exist_ok=True)
    native=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')
    def mask(index):
        record=native['masks'][index];x,y=record['box_top_left'];w,h=record['box_size'];result=np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w]=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'))>0
        return result
    records=[]
    for index,domain,prior,wood in [(76,502,[99,129],99),(93,503,[35,36,37,85,95,101,128,131],35)]:
        original=mask(index);remaining=original.copy();excluded=[]
        for other in prior:
            removed=remaining&mask(other);remaining&=~removed
            excluded.append(dict(native_mask=other,pixels=int(removed.sum()),mask_sha256=sha(OUT/f'baseline/masks/{other:06}.png')))
        # Single-pixel native raster disagreements at existing wood boundaries
        # remain reserved for their existing owner, never new foliage or ground.
        nearwood=np.asarray(Image.fromarray(mask(wood).astype('uint8')*255).filter(ImageFilter.MaxFilter(3)))>0
        boundary=remaining&nearwood;remaining&=~boundary
        file=output/f'domain-{domain}.png';Image.fromarray(remaining.astype('uint8')*255).save(file)
        Image.fromarray(boundary.astype('uint8')*255).save(output/f'{index}-reserved-wood-edge.png')
        x,y=native['masks'][index]['box_top_left'];w,h=native['masks'][index]['box_size'];cut=source.copy();cut.putalpha(Image.fromarray(remaining.astype('uint8')*255));cut.save(output/f'{index}-foliage-full.png');cut.crop((x,y,x+w,y+h)).resize((w*4,h*4),Image.Resampling.NEAREST).save(output/f'{index}-foliage-review.png')
        evidence=output/f'{index}-existing-evidence.json';saved=json.loads(evidence.read_text())
        if sha(Path(saved['worker'])/'model.blend')!=saved['model_sha256']:raise ValueError('Existing reviewed owner changed')
        record=dict(native_mask=index,domain=domain,domain_path=str(file),domain_sha256=sha(file),native_mask_sha256=sha(OUT/f'baseline/masks/{index:06}.png'),native_pixels=int(original.sum()),foliage_pixels=int(remaining.sum()),excluded_prior_domains=excluded,reserved_existing_wood_edge_pixels=int(boundary.sum()),reserved_edge_path=str(output/f'{index}-reserved-wood-edge.png'),existing_wood_owner=saved['asset'],existing_model_sha256=saved['model_sha256'],existing_owner_evidence=str(evidence),existing_owner_evidence_sha256=sha(evidence),native_bbox=[x,y,w,h],status='source split reviewed; foliage geometry pending; no new wood object warranted')
        if int(original.sum())!=int(remaining.sum())+int(boundary.sum())+sum(r['pixels'] for r in excluded):raise ValueError('Incomplete split')
        records.append(record)
    write_json(output/'foliage-splits.json',dict(source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),depth_field_sha256=sha(OUT.parent/'map-compile/occlusion-depth-all-layers/croisement02.layer-0.occlusion-depth.mask-ids.png'),reviewer='Codex /root/missing_fences',records=records,conclusions=['Mask93 contains the existing oak35 base and interleaved foreground foliage; it is not evidence of an independent stump. Saved native-camera render confirms the coherent visible trunk already exists.','Mask76 contains flowering growth layered over the existing wattle99 fence; saved native-camera render confirms its rails/posts already exist. Source-only neutral areas are not absent geometry.','Prior overlapping native domains are excluded from the new foliage assignments. Narrow wood-edge raster discrepancies remain reserved, not assigned as foliage or ground.','Existing oak35 root curtains have visible vertical ribs and thin flared edges; this is a separate geometry-quality issue, not missing object ownership or full-model acceptance.',
        'No approved geometry, UVs, material pixels, catalog ownership, approvals or generated fills were changed. No standalone additive wood candidate is justified by this evidence.']))
    print([(r['native_mask'],r['foliage_pixels'],r['reserved_existing_wood_edge_pixels']) for r in records])

if __name__=='__main__':main()
