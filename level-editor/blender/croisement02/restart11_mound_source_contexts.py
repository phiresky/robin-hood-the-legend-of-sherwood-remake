"""Show the exact initial artwork against its base map at native scale and enlarged."""
from pathlib import Path
import json,hashlib
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=ROOT/'restart11-hiding-mound/closed-support-trio-v1';r=json.loads((worker/'validation.json').read_text());source=json.loads((ROOT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());sprite_path=Path(source['source']);sprite=Image.open(sprite_path).convert('RGBA');base_path=ROOT/'source-states/covered.png';base=Image.open(base_path).convert('RGBA');out=worker/'source-contexts-v1';out.mkdir(exist_ok=False);images=[];panels=[]
 for row in r['records']:
  x,y=row['source_origin'];box=(x-24,y-24,x+sprite.width+24,y+sprite.height+24);before=base.crop(box);after=before.copy();after.alpha_composite(sprite,(24,24));before.save(out/f'{row["tag"]}-base-native.png');after.save(out/f'{row["tag"]}-initial-native.png');panel=Image.new('RGB',(before.width*8+20,before.height*4+130),(35,35,35));draw=ImageDraw.Draw(panel);draw.text((8,8),row['tag']+' | base map / exact initial sprite over base (4x nearest)',fill='white');panel.paste(before.resize((before.width*4,before.height*4),Image.Resampling.NEAREST),(0,30));panel.paste(after.resize((after.width*4,after.height*4),Image.Resampling.NEAREST),(before.width*4+20,30));panel.paste(after,(8,before.height*4+40));draw.text((before.width+20,before.height*4+50),'Native 1:1 pixels. Other mission FX/actors omitted.',fill='white');file=out/f'{row["tag"]}-source-comparison.png';panel.save(file);panels.append(panel);images.append(dict(tag=row['tag'],instance=row['instance'],box=box,source_origin=row['source_origin'],path=file.name,sha256=sha(file)))
 sheet=Image.new('RGB',(max(x.width for x in panels),sum(x.height for x in panels)),(35,35,35));y=0
 for panel in panels:sheet.paste(panel,(0,y));y+=panel.height
 sheet.save(out/'source-context-sheet.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=r['model_sha256'],source_sha256=sha(sprite_path),base_sha256=sha(base_path),images=images,scope='Exact native initial sprite composited on unchanged source map only. Original 1:1 source scale plus4x nearest comparison. Not physical scene render or global mission draw-order proof.'),indent=2)+'\n')
if __name__=='__main__':main()
