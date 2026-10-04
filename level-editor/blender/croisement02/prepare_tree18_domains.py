"""Freeze tree 18's visible wood domain with foreground kindling excluded."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from trace_wood import trace


def main():
    dest=OUT/'tree18-source-revision'
    if (dest/'trace.json').exists():return
    dest.mkdir(exist_ok=True)
    old=OUT/'forest-v4-round-1/assets/croisement02-tree-18'
    m=json.loads((old/'source-masks.json').read_text());inv=json.loads(Path(m['mask_inventory']).read_text())
    bundle=json.loads((OUT/'feedback-source-domains/inventory.json').read_text())
    foreground=next(r for r in bundle['masks'] if r['index']==300);inv['masks'].append(foreground)
    row=next(r for r in inv['masks'] if r['index']==18);x,y=row['box_top_left'];w,h=row['box_size']
    alpha=np.asarray(Image.open(row['png']).convert('L'))>0
    block=np.asarray(Image.open(foreground['png']).convert('L').crop((x,y,x+w,y+h)))>0
    corrected=alpha&~block;Image.fromarray(corrected.astype('uint8')*255).save(dest/'visible-wood.png')
    inv['masks'].append(dict(index=301,layer=0,png=str(dest/'visible-wood.png'),box_top_left=[x,y],box_size=[w,h],provenance='Native wood 18 minus reviewed foreground-kindling domain 300.'))
    (dest/'inventory.json').write_text(json.dumps(inv,indent=2)+'\n');m['mask_inventory']=str(dest/'inventory.json')
    a=next(a for a in m['projections']['exterior']['assignments'] if a.get('asset_group')=='croisement02-tree-18');a['mask_indices']=[301]
    a.update(exclude_mask_indices=[300,135],exclusions_reviewed=True,exclusion_reason='Foreground kindling and native foliage are not trunk bark.')
    (dest/'assignments.json').write_text(json.dumps(m,indent=2)+'\n')
    r=trace(18,corrected);r['method']='Native wood 18 minus foreground kindling 300, then skeleton and distance radii; hidden cross sections inferred.'
    (dest/'trace.json').write_text(json.dumps(r,indent=2)+'\n')

if __name__=='__main__':main()
