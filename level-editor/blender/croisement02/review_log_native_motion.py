"""Provide exact timed native source motion for reviewing trap-body hypotheses."""
import argparse,hashlib,json,math
from pathlib import Path
from PIL import Image,ImageDraw
from catalog import OUT

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--assembly',choices=['log-trap','rock-trap'],default='log-trap');args=parser.parse_args()
    root=OUT/'state-target-evidence'/args.assembly;m=json.loads((root/'manifest.json').read_text());box=m['bbox'];w=box[2]-box[0];h=box[3]-box[1];dest=root/'full-motion';dest.mkdir(exist_ok=True);records=[];previous=None
    for tick in range(m['terminal_geometry_reached_tick']+1):
        canvas=Image.new('RGBA',(w,h));selected=[]
        for part in m['parts']:
            clock=part['start_tick'];frame=part['initial']
            if tick>=clock:
                for frame in part['frames']:
                    clock+=frame['ticks']
                    if tick<clock:break
            x,y=[int(math.floor(a+b+.5))for a,b in zip(part['position'],frame['offset'])];canvas.alpha_composite(Image.open(frame['image']).convert('RGBA'),(x-box[0],y-box[1]));selected.append(frame['image_sha256'])
        digest=hashlib.sha256(canvas.tobytes()).hexdigest()
        if digest==previous:records[-1]['last_tick']=tick;continue
        path=dest/f'{tick:03d}.png';canvas.save(path);records.append(dict(first_tick=tick,last_tick=tick,image=path.name,rgba_sha256=digest,native_frame_sha256=selected));previous=digest
    sheet=Image.new('RGB',(w*2*5,(h*2+28)*math.ceil(len(records)/5)),'#222');draw=ImageDraw.Draw(sheet)
    for i,r in enumerate(records):
        x=(i%5)*w*2;y=(i//5)*(h*2+28);im=Image.open(dest/r['image']);sheet.paste(im.resize((w*2,h*2),Image.Resampling.NEAREST),(x,y+28),im.resize((w*2,h*2),Image.Resampling.NEAREST));draw.text((x+5,y+5),f"ticks {r['first_tick']}–{r['last_tick']} / {r['first_tick']/25:.2f}s",fill='white')
    sheet.save(dest/'storyboard.png');(dest/'manifest.json').write_text(json.dumps(dict(tick_rate=25,terminal_tick=m['terminal_geometry_reached_tick'],records=records,scope='Exact native composited source evidence; not a geometry animation or proof of rigid-body identity.'),indent=2)+'\n')
    html='''<!doctype html><meta charset="utf-8"><title>Native log-trap motion evidence</title><style>body{font:16px system-ui;background:#222;color:white;margin:24px}img{image-rendering:pixelated;width:768px;max-width:95vw}input{width:600px;max-width:80vw}</style><h1>Native log-trap motion evidence</h1><p>Exact native sprites at25 ticks/s. This is source evidence, not a3D animation. Background/shadow receivers remain separate.</p><p><button id="play">Play / pause</button> <input id="tick" type="range" min="0" max="TERMINAL" value="0"> <span id="label"></span></p><img id="frame"><script>const data=RECORDS;let playing=false,t=0;function show(){t=Number(document.querySelector('#tick').value);let r=data.find(x=>t>=x.first_tick&&t<=x.last_tick);document.querySelector('#frame').src=r.image;document.querySelector('#label').textContent=`tick ${t} / ${(t/25).toFixed(2)}s`;}document.querySelector('#tick').oninput=show;document.querySelector('#play').onclick=()=>playing=!playing;setInterval(()=>{if(playing){document.querySelector('#tick').value=t>=TERMINAL?0:t+1;show();}},40);show();</script>'''.replace('RECORDS',json.dumps(records)).replace('TERMINAL',str(m['terminal_geometry_reached_tick'])).replace('log-trap',args.assembly);(dest/'index.html').write_text(html);print(len(records),'unique source phases')
if __name__=='__main__':main()
