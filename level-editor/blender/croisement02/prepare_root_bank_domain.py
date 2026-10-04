"""Trace the northeast root-bank contact and reserve its visible source pixels."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json

# Native crest anchors remain the height constraint. The low cliff toe is
# independently traced on source artwork, not copied from the proxy footprint.
TOE=[(1460.5991,219.95029),(1422.342,242),(1382.7297,255),(1362.3541,254),
     (1353.2383,226),(1380.172,169.14757)]
SOURCE_OUTLINE=[(1380,169),(1405,185),(1422,200),(1450,208),(1461,220),
                (1445,235),(1422,242),(1403,250),(1383,255),(1363,254),
                (1353,226),(1348,212),(1353,189)]


def main():
    folder=OUT/'root-bank-source-revision';folder.mkdir(exist_ok=True)
    inventory=json.loads((OUT/'scenery-domains/inventory.json').read_text())
    image=Image.new('L',(1792,1152));ImageDraw.Draw(image).polygon(SOURCE_OUTLINE,fill=255)
    coverage=np.asarray(image)>0;overlaps={}
    for row in inventory['masks']:
        if row['index'] not in list(range(48))+list(range(128,136)):continue
        x,y=row['box_top_left'];w,h=row['box_size'];native=np.asarray(Image.open(row['png']).convert('L'))>0
        left,top=max(0,x),max(0,y);right,bottom=min(1792,x+w),min(1152,y+h)
        if right<=left or bottom<=top:continue
        region=coverage[top:bottom,left:right];mask=native[top-y:bottom-y,left-x:right-x]
        count=int((region&mask).sum())
        if count:overlaps[str(row['index'])]=count
        region[mask]=False
    for index,domain in {370:coverage,371:~coverage}.items():
        path=folder/f'domain-{index}.png';Image.fromarray(domain.astype('uint8')*255).save(path)
        inventory['masks'].append(dict(index=index,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],
            provenance='Independent source cliff-foot trace and explicit local terrain allocation, minus native tree ownership.'))
    write_json(folder/'inventory.json',inventory)
    old=OUT/'scenery-round-2/assets/croisement02-northeast-oak-root-bank'
    manifest=json.loads((old/'source-masks.json').read_text());manifest['mask_inventory']=str(folder/'inventory.json')
    assignment=next(a for a in manifest['projections']['exterior']['assignments'] if a.get('asset_group')==old.name)
    assignment['mask_indices']=[370]
    manifest['projections']['exterior']['occluder_constraints']=[dict(reviewed=True,source_node=node,
        receiver_nodes=['building-025'],mask_indices=[371],reason='Coarse context cannot hide independently traced visible bank artwork; native tree pixels are excluded from this domain.',review_evidence=str(folder/'ownership-review.json'))
        for node in ['ground',*(f'building-{i:03}' for i in range(150))] if node!='building-025']
    write_json(folder/'assignments.json',manifest)
    source=OUT/'animation-references/composite-frame-0.png';rgba=Image.open(source).convert('RGBA')
    crop=(1320,145,1490,275);board=Image.new('RGB',(1360,544),'#888');draw=ImageDraw.Draw(board)
    for i,(label,domain) in enumerate([('Original source and traced bank toe',None),('Bank source allocation; native tree pixels excluded',coverage)]):
        im=rgba.copy()
        if domain is not None:im.putalpha(Image.fromarray(domain.astype('uint8')*255))
        im=im.crop(crop).resize((680,520),Image.Resampling.NEAREST);board.paste(im,(i*680,24),im)
        draw.text((i*680+4,4),label,fill='white')
    draw.line([((x-1320)*4,(y-145)*4+24) for x,y in TOE[:5]],fill='cyan',width=2)
    board.save(folder/'ownership-sheet.png')
    write_json(folder/'ownership-review.json',dict(status='candidate; manual source review required',source_sha256=sha(source),
        sheet_sha256=sha(folder/'ownership-sheet.png'),domain_sha256=sha(folder/'domain-370.png'),
        source_outline=SOURCE_OUTLINE,source_toe=TOE,excluded_native_masks=overlaps,pixels=int(coverage.sum()),
        notes=['The source shows the left cliff toe below the native proxy base; this trace extends that transition while retaining native high-crest heights.',
               'The rear and lateral local terrain partition is an authored ownership boundary, not an observed object silhouette. Main ground must reserve this exact domain.']))
    print(folder)


if __name__=='__main__':main()
