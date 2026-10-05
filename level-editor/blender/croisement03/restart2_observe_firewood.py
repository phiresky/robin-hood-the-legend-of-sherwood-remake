"""Apply visible billet-end constraints to the coarse silhouette fit."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement/restart2'

def main():
    result=OUT/'firewood-fit-v5';result.mkdir(parents=True,exist_ok=False)
    record=json.loads((OUT/'firewood-fit-v1/fit.json').read_text());vertices=np.array(record['vertices'])
    for i,(shorten_x,lower_y) in enumerate([(12.5,4.2),(7.5,3.2),(0.,0.)]):
        log=record['logs'][i];a=np.array(log['a']);b=np.array(log['b']);axis=(b-a)/np.linalg.norm(b-a);delta=-axis*(shorten_x/axis[0]);delta[1]-=lower_y/math.sin(math.radians(35));vertices[i*32+16:(i+1)*32]+=delta;log['b']=(b+delta).tolist()
    record['vertices']=vertices.tolist();record['source_fit']=dict(status='Silhouette fit superseded by source-visible end constraints; final native renderer audit required')
    record['source_observations']=['Dominant upper diagonal billet remains longest.','Near lower billet ends around435,784; the pale end is below the dark cap center.','Second lower billet ends around445,782.','Three billets remain an inferred count; dark and green mask114 end regions are not automatically timber.']
    (result/'fit.json').write_text(json.dumps(record,indent=2)+'\n')
    image=Image.open(OUT.parent/'baseline/covered.png').convert('RGB').crop((390,742,470,798)).resize((800,560),Image.Resampling.NEAREST);draw=ImageDraw.Draw(image)
    for number,(x,y) in enumerate([(435,784),(445,782),(447,777)]):
        cx,cy=(x-390)*10,(y-742)*10;draw.ellipse((cx-6,cy-6,cx+6,cy+6),outline='magenta',width=2);draw.text((cx+7,cy),str(number),fill='white',stroke_width=1,stroke_fill='black')
    image.save(result/'end-observations.png')

if __name__=='__main__':main()
