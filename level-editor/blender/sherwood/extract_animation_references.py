"""Extract authored Sherwood FX references; run with normal Python/Pillow.

Keeps animation frame offsets, source delays and separate assets. The composite
is a synchronized frame-zero reference, not an exact runtime-phase screenshot.
"""

import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
DATA=ROOT/'datadirs/fullgame_gog_hackable/Data'
OUT=ROOT/'level-editor/work/sherwood-refinement/animation-references'
OUT.mkdir(parents=True,exist_ok=True)
level=json.loads((DATA/'Levels/Sherwood.rhp.json').read_text())
composite=Image.open(DATA/'Levels/Day/sherwood.map.png').convert('RGBA')
records=[]


def keyed_image(path):
    pixels=np.array(Image.open(path).convert('RGBA'))
    key=np.all(pixels[:,:,:3]==[0,251,0],axis=2)
    pixels[key]=[0,0,0,0]
    return Image.fromarray(pixels)


for index,item in enumerate(level['animations']):
    if not item['active']:
        continue
    sprite=item['sprite']
    bank=DATA/'Animations/Day'/f"{sprite['frame_profile_name']}.rhs.d"
    manifest=json.loads((bank/'manifest.json').read_text())
    profile=next(p for p in manifest['profiles'] if p['name']==sprite['profile_name'])
    row=profile['rows'][0]
    slug=profile['name'].split(' - ',1)[1].lower()
    frames=row['frames']
    paths=[bank/profile['name']/row['path']/f['file'] for f in frames]
    if any(not p.is_file() for p in paths):
        raise FileNotFoundError(f'Missing animation frame in {profile["name"]}')
    first=keyed_image(paths[0])
    left=round(sprite['position_x']+frames[0]['offset_x'])
    top=round(sprite['position_y']+frames[0]['offset_y'])
    first.save(OUT/f'{slug}-first.png')
    composite.alpha_composite(first,(left,top))
    record={'index':index,'profile':profile['name'],'bank':sprite['frame_profile_name'],
            'kind':'tree' if sprite['frame_profile_name']=='shertree' else 'ambient',
            'first_png':f'{slug}-first.png','left':left,'top':top,'size':list(first.size),
            'position':[sprite['position_x'],sprite['position_y']],
            'center':[profile['center_x'],profile['center_y']],
            'elevation':sprite['elevation'],'force_display':item['force_display'],
            'blit_type':item['blit_type'],'frame_count':len(frames),
            'delays':[f['delay'] for f in frames],
            'offsets':[[f['offset_x'],f['offset_y']] for f in frames],
            'source_frames':[str(p.relative_to(ROOT)) for p in paths]}
    if record['kind']=='tree':
        loaded=[keyed_image(p) for p in paths]
        minx=math.floor(min(f['offset_x'] for f in frames))
        miny=math.floor(min(f['offset_y'] for f in frames))
        maxx=math.ceil(max(f['offset_x']+im.width for f,im in zip(frames,loaded)))
        maxy=math.ceil(max(f['offset_y']+im.height for f,im in zip(frames,loaded)))
        width,height=maxx-minx,maxy-miny
        if len(frames)!=16 or any(f['delay']!=3 for f in frames):
            raise ValueError('Expected sixteen tree frames, each with three wait ticks')
        gutter=2
        atlas=Image.new('RGBA',(4*(width+2*gutter),4*(height+2*gutter)))
        for i,(frame,im) in enumerate(zip(frames,loaded)):
            x=(i%4)*(width+2*gutter)+gutter+round(frame['offset_x'])-minx
            y=(i//4)*(height+2*gutter)+gutter+round(frame['offset_y'])-miny
            atlas.alpha_composite(im,(x,y))
        atlas.save(OUT/f'{slug}-atlas.png')
        record.update({'atlas':f'{slug}-atlas.png','canvas':[width,height],
                       'canvas_offset':[minx,miny],'atlas_size':list(atlas.size),
                       'gutter':gutter,'ticks_per_frame':4,'tick_hz':25})
    records.append(record)

composite.save(OUT/'sherwood-composite-frame00.png')
(OUT/'manifest.json').write_text(json.dumps({'reference':'Synchronized frame-zero authored overlays; no actors or runtime sorting',
    'placement_rule':'screen top-left = stored position + frame offset; elevation cancels in projection',
    'assets':records},indent=2))
trees=[r for r in records if r['kind']=='tree']
sheet=Image.new('RGB',(1200,900),(32,35,38))
draw=ImageDraw.Draw(sheet)
for i,record in enumerate(sorted(trees,key=lambda r:r['profile'])):
    im=Image.open(OUT/record['first_png'])
    im.thumbnail((380,245))
    x,y=(i%3)*400,(i//3)*450
    checker=Image.new('RGBA',(390,350),(75,75,75,255))
    cd=ImageDraw.Draw(checker)
    for yy in range(0,350,16):
        for xx in range(0,390,16):
            if ((xx//16)+(yy//16))%2:cd.rectangle((xx,yy,xx+15,yy+15),fill=(100,100,100,255))
    checker.alpha_composite(im,((390-im.width)//2,(350-im.height)//2))
    sheet.paste(checker.convert('RGB'),(x,y+35))
    draw.text((x+8,y+8),record['profile'],fill='white')
    draw.text((x+8,y+395),f"16 frames; elevation {record['elevation']}; position {record['position']}",fill='white')
sheet.save(OUT/'tree-contact-sheet.png')
print(json.dumps({'assets':len(records),'trees':len(trees),'output':str(OUT)}))
