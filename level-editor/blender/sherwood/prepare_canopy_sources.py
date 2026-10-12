"""Freeze original animated canopy canvases and their exact receiver families."""
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from source_authority import sha

EDITOR=Path(__file__).resolve().parents[2]
BASE=EDITOR/'work/needs-review/sherwood-refinement'

def main(output):
    output=Path(output).resolve();output.mkdir(parents=True,exist_ok=False)
    reference=BASE/'animation-references/manifest.json'
    geometry=BASE/'grouping-review/geometry.json'
    objects=json.loads(geometry.read_text())
    records=[];contact=Image.new('RGB',(1200,800),'#17202b');draw=ImageDraw.Draw(contact)
    for i,record in enumerate(r for r in json.loads(reference.read_text())['assets'] if r['kind']=='tree'):
        profile=record['profile'];sprite=reference.parent/record['first_png'];im=Image.open(sprite).convert('RGBA')
        canvas=Image.new('RGBA',(max(1920,record['left']+im.width),max(1088,record['top']+im.height)))
        canvas.paste(im,(record['left'],record['top']))
        path=output/(profile.rsplit(' - ',1)[-1].lower()+'.png');canvas.save(path)
        alpha=(np.asarray(canvas)[...,3]>=128).astype('uint8')*255
        mask=path.with_name(path.stem+'-mask.png');Image.fromarray(alpha).save(mask)
        names=[r['name'] for r in objects if r['name'].startswith(profile+' tree') and ('individual leaf sprays' in r['name'] or 'inner foliage masses' in r['name'])]
        if not names:raise ValueError('No receivers for '+profile)
        records.append(dict(profile=profile,source=str(path),source_sha256=sha(path),mask=str(mask),mask_sha256=sha(mask),sprite=str(sprite),sprite_sha256=sha(sprite),placement=[record['left'],record['top']],receivers=names))
        thumb=im.copy();thumb.thumbnail((390,350));x=i%3*400;y=i//3*400
        draw.text((x+10,y+10),profile,fill='white');contact.paste(thumb,(x+(400-thumb.width)//2,y+40),thumb)
    contact.save(output/'source-contact.png')
    (output/'manifest.json').write_text(json.dumps(dict(status='ORIGINAL_LAYER_DOMAINS',layers=records,evidence={str(reference):sha(reference),str(geometry):sha(geometry)},source_contact_sha256=sha(output/'source-contact.png')),indent=2)+'\n')
    print('Prepared',len(records),'original canopy layers;',sum(len(r['receivers']) for r in records),'leaf receivers')

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--output',required=True);main(p.parse_args().output)
