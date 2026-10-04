"""Separate the southwest rock source from its independently owned foliage."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json


def main():
    destination = OUT / 'southwest-rock-source-revision'
    destination.mkdir(exist_ok=True)
    inventory = json.loads((OUT / 'scenery-domains/inventory.json').read_text())
    rows = {r['index']:r for r in inventory['masks']}
    def mask(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size']
        result=np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w]=np.asarray(Image.open(row['png']).convert('L'))>0
        return result
    rock=mask(52)&~mask(81)&~mask(83)
    if (rock&mask(106)).any():raise ValueError('Approved stump pixels overlap rock')
    for index,domain in {360:rock,361:~rock}.items():
        path=destination/f'domain-{index}.png'
        Image.fromarray(domain.astype('uint8')*255).save(path)
        inventory['masks'].append(dict(index=index,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],
            provenance='Native52 minus foreground81 and back-edge foliage83; stump106 is disjoint.'))
    write_json(destination/'inventory.json',inventory)
    old=OUT/'scenery-round-2/assets/croisement02-southwest-rock-outcrop'
    manifest=json.loads((old/'source-masks.json').read_text())
    manifest['mask_inventory']=str(destination/'inventory.json')
    assignment=next(r for r in manifest['projections']['exterior']['assignments'] if r.get('asset_group')==old.name)
    assignment['mask_indices']=[360]
    for key in ('exclude_mask_indices','exclusions_reviewed','exclusion_reason'):assignment.pop(key,None)
    receivers=[f'building-{i:03}' for i in [43,131,136,137]]
    manifest['projections']['exterior']['occluder_constraints']=[dict(reviewed=True,source_node=node,
        receiver_nodes=receivers,mask_indices=[361],reason='Unrelated coarse context blocks only outside the independently reviewed visible rock domain.',
        review_evidence=str(destination/'ownership-review.json'))
        for node in ['ground',*(f'building-{i:03}' for i in range(150))] if node not in receivers]
    write_json(destination/'assignments.json',manifest)
    source=OUT/'animation-references/composite-frame-0.png';rgba=Image.open(source).convert('RGBA')
    sheet=Image.new('RGB',(1040,468),'#888');draw=ImageDraw.Draw(sheet)
    for i,(name,domain) in enumerate([('Native source context',None),('Rock52 minus81 and83; stump106 excluded naturally',rock)]):
        im=rgba.copy()
        if domain is not None:im.putalpha(Image.fromarray(domain.astype('uint8')*255))
        im=im.crop((250,810,510,1032)).resize((520,444),Image.Resampling.NEAREST)
        sheet.paste(im,(i*520,24),im);draw.text((i*520+4,4),name,fill='white')
    sheet.save(destination/'ownership-sheet.png')
    write_json(destination/'ownership-review.json',dict(status='candidate; visual review required',source_sha256=sha(source),
        sheet_sha256=sha(destination/'ownership-sheet.png'),rock_pixels=int(rock.sum()),
        excluded_foreground81_pixels=int((mask(52)&mask(81)).sum()),excluded_foliage83_pixels=int((mask(52)&mask(83)).sum()),
        approved_stump106_overlap_pixels=0,domain_hashes={str(i):sha(destination/f'domain-{i}.png') for i in [360,361]}))
    print(destination)


if __name__=='__main__':main()
