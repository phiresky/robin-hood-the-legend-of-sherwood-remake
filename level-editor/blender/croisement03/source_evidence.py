"""Freeze Croisement03 animation references and explicit source-mask candidates."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement'
DATA=ROOT/'datadirs/fullgame_gog_hackable/Data'

def main():
    destination=OUT/'animation-references';destination.mkdir(exist_ok=False)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    composite=Image.open(OUT/'baseline/covered.png').convert('RGBA')
    records=[]
    for index,entry in enumerate(level['animations']):
        if not entry['active']:continue
        sprite=entry['sprite'];bank=DATA/'Animations/Day'/f"{sprite['frame_profile_name']}.rhs.d"
        profiles=json.loads((bank/'manifest.json').read_text())['profiles']
        profile=next(p for p in profiles if p['name']==sprite['profile_name'])
        row=profile['rows'][0];frames=[]
        folder=destination/f'animation-{index:02}';folder.mkdir()
        for number,frame in enumerate(row['frames']):
            source=bank/profile['name']/row['path']/frame['file']
            rgba=np.asarray(Image.open(source).convert('RGBA')).copy()
            rgba[np.all(rgba[:,:,:3]==[0,251,0],axis=2)]=0
            image=Image.fromarray(rgba);target=folder/f'{number:03}.png';image.save(target)
            left=round(sprite['position_x']+frame['offset_x']);top=round(sprite['position_y']+frame['offset_y'])
            if number==0:composite.alpha_composite(image,(left,top))
            frames.append(dict(image=str(target),bbox=[left,top,*image.size],delay=frame['delay'],
                               source=str(source),sha256=hashlib.sha256(source.read_bytes()).hexdigest()))
        records.append(dict(index=index,kind='unclassified-native-animation',profile=profile['name'],
                            sprite=sprite,frames=frames,display_polyline=entry['display_polyline']))
    composite.save(destination/'composite-frame-0.png')
    (destination/'manifest.json').write_text(json.dumps({'state':'Synchronized first-frame reference; runtime phases and sorting are not simulated','animations':records},indent=2)+'\n')
    print('Extracted',len(records),'animation references')

if __name__=='__main__':main()
