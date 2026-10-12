"""Prepare deterministic native artwork comparisons for the completed foliage family."""
import json,sys
from pathlib import Path
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q

def main():
    dest=q.OUT/'profile04-native-context-v1';assert not dest.exists();dest.mkdir()
    qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][3];background=q.ROOT/qualification['background']['path'];assert q.pin(background)==qualification['background'];box=[-20,846,164,977];base=Image.open(background).convert('RGBA').crop(box);sheet=Image.new('RGB',(368,302),(32,32,32));draw=ImageDraw.Draw(sheet);records=[]
    for i,(phase,frame)in enumerate(zip(['initial','transition-0','transition-1','applied'],row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames'])):
        image=base.copy();image.alpha_composite(q.rgba(frame),(int(frame['top_left'][0])-box[0],int(frame['top_left'][1])-box[1]));path=dest/(phase+'.png');image.save(path);x=(i%2)*184;y=(i//2)*151;sheet.paste(image.convert('RGB'),(x,y+20));draw.text((x+4,y+4),'Native art: '+phase,fill='white');records.append(dict(phase=phase,image=q.pin(path),source=frame['image']))
    sheet.save(dest/'native-states.png');(dest/'report.json').write_text(json.dumps(dict(status='NATIVE_SOURCE_CONTEXT',background=qualification['background'],box=box,states=records,sheet=q.pin(dest/'native-states.png'),limits=['Exact native state artwork alpha-composited over the qualified background.','Off-map crop area has no observed artwork; no inferred geometry is shown here.']),indent=2)+'\n')
if __name__=='__main__':main()
