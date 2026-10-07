"""Freeze one real actor frame and independent scalar packed-mask expectations."""
import hashlib,json,pathlib,struct
from PIL import Image
ROOT=pathlib.Path(__file__).resolve().parents[4]
OUT=ROOT/'level-editor/work/croisement02-refinement/restart20-hidden-outline-v1'
BASE=ROOT/'datadirs/fullgame_gog_hackable/Data'
def read(p):return json.loads(p.read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
mission=read(BASE/'Levels/S03_FoB_MP.rhm.json');entity=mission['soldiers'][0]
level_path=BASE/'Levels/Croisement02.rhp.json';level=read(level_path)
image_path=BASE/'Characters/Archer01.rhs.d/WaitingUpright/dir_06/00.png'
with Image.open(image_path) as im:
    rgba=list(im.convert('RGBA').tobytes());width,height=im.size
source=[]
for i in range(0,len(rgba),4):
    r,g,b,a=rgba[i:i+4];v=(r>>3)<<11|(g>>2)<<5|b>>3
    source.append(0x7c0 if a==0 else v)
def decode(mask):
    bits=[];offset=0;data=mask['mask_data'];w,h=mask['box_size']
    for y in range(h):
        end=offset+1+data[offset];offset+=1;row=[]
        while offset<end:
            c=data[offset];offset+=1;n=c&127
            if c&128:values=[data[offset]]*n;offset+=1
            else:values=data[offset:offset+n];offset+=n
            for v in values:row.extend((v>>b)&1 for b in range(7,-1,-1))
        bits.extend(row[:w]+[0]*max(0,w-len(row)))
    return bits
def apply(pixels,origin,mask):
    mx,my=mask['box_top_left'];mw,mh=mask['box_size'];sx,sy=origin
    x0=max(mx,sx);x1=min(mx+mw,sx+width);y0=max(my,sy);y1=min(my+mh,sy+height)
    bits=decode(mask);out=pixels[:];edges=0
    for y in range(y0,y1):
        for x in range(x0,x1):
            if bits[(y-my)*mw+x-mx]:
                i=(y-sy)*width+x-sx;a=out[i];b=out[i+1] if x<x1-1 else 0x7c0
                a=0x7c0 if a==31 else a;b=0x7c0 if b==31 else b
                edge=x<x1-1 and a!=b and (a==0x7c0 or b==0x7c0)
                out[i]=0xf800 if edge else 0x7c0;edges+=edge
    return out,edges
# A source-art contact fixture deliberately translates the sprite across one real
# mask. It does not claim this artificial contact is an authored actor placement.
best=None
for idx,mask in enumerate(level['masks']):
    if not mask['mask_type']&1:continue
    mx,my=mask['box_top_left'];mw,mh=mask['box_size']
    origin=[mx+mw//2-width//2,my+mh//2-height//2]
    result,edges=apply(source,origin,mask)
    if edges and (best is None or edges>best[0]):best=(edges,idx,origin,result)
edges,idx,origin,result=best;mask=level['masks'][idx];foot=mask['character_polyline'][0]
fixture={'scope':'Real source actor pixels with deliberately translated contact against one real native mask; not mission-placement parity',
 'image':{'path':str(image_path.relative_to(ROOT)),'sha256':sha(image_path)},'level':{'path':str(level_path.relative_to(ROOT)),'sha256':sha(level_path)},
 'source':{'width':width,'height':height,'data':rgba},'maskIndex':idx,'screenOrigin':origin,
 'actor':{'layer':mask['layer'],'mapPosition':[foot[0],foot[1]-1]},'expectedOutlined':edges,
 'expectedPackedSha256':hashlib.sha256(struct.pack('<'+'H'*len(result),*result)).hexdigest()}
OUT.mkdir(exist_ok=True);(OUT/'fixture.json').write_text(json.dumps(fixture)+'\n')
print(json.dumps({'mask':idx,'outlined':edges,'bytes':(OUT/'fixture.json').stat().st_size}))
