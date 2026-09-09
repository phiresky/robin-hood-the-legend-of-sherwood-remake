"""Source-camera registration report; pixel error is not geometry fidelity."""
import json
import numpy as np
from PIL import Image, ImageDraw
from paths import DATA, OUT

original=np.array(Image.open(DATA/'Levels/Day/sherwood.map.png').convert('RGB'),dtype=float)
regions={'full':(0,0,1920,1088),'central_oak':(870,160,1190,750),
         'river':(1370,400,1920,1030),'ladder_oak':(385,150,480,450),'camp':(370,520,690,840)}
report={'meaning':'Mean absolute RGB error on 0-255 channels; registration, not percent 3D fidelity'}
for name in ['baseline-bare-map','refined-bare-map']:
    im=np.array(Image.open(OUT/(name+'.png')).convert('RGB'),dtype=float)
    if im.shape!=original.shape:raise ValueError('Reference and render dimensions differ')
    report[name]={key:float(np.abs(im[y:Y,x:X]-original[y:Y,x:X]).mean()) for key,(x,y,X,Y) in regions.items()}
a=np.array(Image.open(OUT/'animation-01.png').convert('RGB'),dtype=float)
b=np.array(Image.open(OUT/'animation-33.png').convert('RGB'),dtype=float)
report['animation_changed_pixels']=int(np.any(a!=b,axis=2).sum())
(OUT/'image-comparison.json').write_text(json.dumps(report,indent=2))
source=Image.open(OUT.parent/'animation-references/sherwood-composite-frame00.png').convert('RGB')
refined=Image.open(OUT/'refined-composite.png').convert('RGB')
sheet=Image.new('RGB',(1440,850),(27,29,31));draw=ImageDraw.Draw(sheet)
for image,x,label in [(source,0,'Original map + authored frame-zero overlays'),(refined,720,'Refined Blender scene - same camera')]:
    sheet.paste(image.resize((720,408)),(x,32));draw.text((x+12,12),label,fill='white')
for i,(label,box) in enumerate([('Central oak',(890,210,1140,480)),('Camp',(380,745,540,835)),('River',(1460,640,1690,850))]):
    for j,image in enumerate([source,refined]):
        crop=image.crop(box);crop.thumbnail((225,345));x=i*480+j*235+10
        sheet.paste(crop,(x,480));draw.text((x,455),label+(' source' if j==0 else ' Blender'),fill='white')
sheet.save(OUT/'reference-comparison.png')
print(json.dumps(report,indent=2))
