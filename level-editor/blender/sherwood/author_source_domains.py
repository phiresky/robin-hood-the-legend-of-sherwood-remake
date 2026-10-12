"""Build explicit hand-traced source partitions, retaining native silhouette pixels.

Coordinates are local to the named native mask. These are candidate assignments
until their side-by-side source cutouts have been inspected and hash-bound.
"""
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from build_mask_review import bitmap, digest, INVENTORY, SOURCE, EDITOR

ROOT=EDITOR/'work/needs-review/sherwood-refinement/textures/native-mask-audit/authored-domains'
SPEC=Path(__file__).with_name('source-domain-partitions.json')

def main():
    spec=json.loads(SPEC.read_text()); native={r['index']:r for r in json.loads(INVENTORY.read_text())['masks']}
    original=Image.open(SOURCE).convert('RGB'); masks={i:bitmap(r,original.size) for i,r in native.items()}
    ROOT.mkdir(parents=True,exist_ok=True); rows=[]; authored={}
    for rule in spec['domains']:
        origin=rule.get('coordinate_origin', native[rule['coordinate_mask']]['box_top_left'] if 'coordinate_mask' in rule else [0,0])
        allowed=np.logical_or.reduce([masks[i] for i in rule['native_masks']]) if rule.get('clip_native',True) else np.ones((original.height,original.width),dtype=bool)
        for index in rule.get('exclude_native_masks',[]):
            excluded=Image.fromarray(masks[index].astype('uint8')*255)
            if rule.get('native_exclusion_margin',0):excluded=excluded.filter(ImageFilter.MaxFilter(2*rule['native_exclusion_margin']+1))
            allowed &= np.asarray(excluded)==0
        if rule.get('include_polygons'):
            domain=Image.new('L',original.size);draw=ImageDraw.Draw(domain)
            for poly in rule['include_polygons']:draw.polygon([(x+origin[0],y+origin[1]) for x,y in poly],fill=255)
            allowed &= np.asarray(domain)>0
        if rule.get('include_lines'):
            domain=Image.new('L',original.size);draw=ImageDraw.Draw(domain)
            for line in rule['include_lines']:draw.line([(x+origin[0],y+origin[1]) for x,y in line['points']],fill=255,width=line['width'])
            allowed &= np.asarray(domain)>0
        if rule.get('exclude_polygons'):
            domain=Image.new('L',original.size);draw=ImageDraw.Draw(domain)
            for poly in rule['exclude_polygons']:draw.polygon([(x+origin[0],y+origin[1]) for x,y in poly],fill=255)
            if rule.get('exclusion_margin',0):domain=domain.filter(ImageFilter.MaxFilter(2*rule['exclusion_margin']+1))
            allowed &= np.asarray(domain)==0
        for id in rule.get('exclude_domains',[]):
            excluded=Image.fromarray(authored[id].astype('uint8')*255)
            if rule.get('domain_exclusion_margin',0):excluded=excluded.filter(ImageFilter.MaxFilter(2*rule['domain_exclusion_margin']+1))
            allowed &= np.asarray(excluded)==0
        if rule.get('known_absent'):
            allowed[:]=False
        authored[rule['id']]=allowed.copy()
        output=ROOT/(rule['id']+'.png');Image.fromarray(allowed.astype('uint8')*255).save(output)
        native_union=np.logical_or.reduce([masks[i] for i in rule['native_masks']]) if rule['native_masks'] else allowed
        if not (native_union | allowed).any():
            if not rule.get('known_absent') or not rule.get('review_box'):raise ValueError('Unexplained empty source domain')
            native_union=np.zeros_like(allowed);x0,y0,x1,y1=rule['review_box'];native_union[y0:y1,x0:x1]=True
        ys,xs=np.where(native_union | allowed)
        box=(max(0,int(xs.min())-4),max(0,int(ys.min())-4),min(original.width,int(xs.max())+5),min(original.height,int(ys.max())+5))
        box=tuple(rule.get('review_box',box))
        cut=Image.new('RGB',original.size,'#17202b');cut.paste(original,(0,0),Image.fromarray(allowed.astype('uint8')*255))
        scale=min(3,600/(box[2]-box[0]),750/(box[3]-box[1]));size=(round((box[2]-box[0])*scale),round((box[3]-box[1])*scale))
        card=Image.new('RGB',(size[0]*2,size[1]+40),'#272e38');draw=ImageDraw.Draw(card);draw.text((5,5),rule['id']+' | Original / assigned pixels',fill='white')
        for i,im in enumerate([original,cut]):card.paste(im.crop(box).resize(size,Image.Resampling.NEAREST),(size[0]*i,40))
        review=ROOT/(rule['id']+'-review.png');card.save(review)
        rows.append(dict(**rule,mask=str(output),mask_sha256=digest(output),card=str(review),card_sha256=digest(review),accepted_pixels=int(allowed.sum())))
    (ROOT/'manifest.json').write_text(json.dumps(dict(status='CANDIDATE',source_sha256=digest(SOURCE),inventory_sha256=digest(INVENTORY),spec_sha256=digest(SPEC),domains=rows),indent=2)+'\n')
    print('Built',len(rows),'explicit source domains')

if __name__=='__main__':main()
