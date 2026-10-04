"""Align saved native-camera sign renders with native non-shadow artwork."""
import sys,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json


def main():
    directory=Path(sys.argv[1]);evidence=json.loads((directory/'evidence.json').read_text());frames=evidence['profile']['rows'][0]['frames'];left=-35;top=-math.cos(math.radians(35))*23-35
    sheet=Image.new('RGB',(1120,1200),(70,70,70));draw=ImageDraw.Draw(sheet);overlay_sheet=Image.new('RGB',(1120,600),(70,70,70));od=ImageDraw.Draw(overlay_sheet);solid_sheet=Image.new('RGB',(1120,600),(210,210,210));sd=ImageDraw.Draw(solid_sheet);rows=[]
    for i,index in enumerate(range(0,32,4)):
        frame=frames[index];raw=Image.open(frame['image']).convert('RGBA');a=np.asarray(raw).copy();yy=np.indices(a.shape[:2])[0]+frame['offset'][1];a[(a[:,:,:3].max(axis=2)<=12)&(yy>=0),3]=0;raw=Image.fromarray(a);ox,oy=frame['offset'];native=raw.transform((280,280),Image.Transform.AFFINE,(.25,0,left-ox,0,.25,top-oy),resample=Image.Resampling.NEAREST)
        actual=Image.open(directory/f'actual-pose-{index:02}.png').convert('RGBA');known=np.asarray(native)[:,:,3]>127;modeled=np.asarray(actual)[:,:,3]>127
        rows.append(dict(frame=index,source_sha256=sha(Path(frame['image'])),render_sha256=sha(directory/f'actual-pose-{index:02}.png'),native_nonshadow_pixels=int(known.sum()),rendered_pixels=int(modeled.sum()),intersection=int((known&modeled).sum()),missing=int((known&~modeled).sum()),extra=int((modeled&~known).sum()),iou=float((known&modeled).sum()/max(1,(known|modeled).sum()))))
        x=i%2*560;y=i//2*300;sheet.paste(native,(x,y+20),native);sheet.paste(actual,(x+280,y+20),actual);draw.text((x,y),f'Native{index}; black shadow excluded',fill='white');draw.text((x+280,y),f'Closed geometry{index}',fill='white')
        overlay=Image.new('RGBA',(280,280),(70,70,70,255));overlay.alpha_composite(native);tint=actual.copy();tint.putalpha(tint.getchannel('A').point(lambda v:v*.45));overlay.alpha_composite(tint);x=i%4*280;y=i//4*300;overlay_sheet.paste(overlay,(x,y+20));od.text((x,y),f'Native/geometry{index}',fill='white')
        solid=Image.open(directory/f'solid-pose-{index:02}.png').convert('RGBA');solid_sheet.paste(solid,(x,y+20),solid);sd.text((x,y),f'Closed solid{index}',fill='black')
    sheet.save(directory/'native-comparison.png');overlay_sheet.save(directory/'native-overlay.png');solid_sheet.save(directory/'solid8-light-background.png')
    write_json(directory/'source-comparison.json',dict(model_sha256=sha(directory/'model.blend'),rows=rows,limitation='Exact review camera alignment; dark native shadow pixels excluded. Pixel fit diagnoses a physical hypothesis, not arbitrary silhouette extrusion. Rotation phase/dimensions may require refinement.',source_evidence_sha256=sha(directory/'evidence.json')))
    print([(r['frame'],round(r['iou'],3)) for r in rows])

if __name__=='__main__':main()
