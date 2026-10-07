"""Source and full-pose presentation for the private sign ordering experiment."""
from pathlib import Path
import hashlib,json,math
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]/'level-editor/work/croisement02-refinement'
BASE=ROOT/'restart10-physical-signs'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 source=BASE/'ordering-review-v2';out=BASE/'ordering-presentation-v1';out.mkdir(exist_ok=False);files={}
 for target in range(4,9):
  grid=Image.new('RGB',(4*256,8*276),(45,45,45));gd=ImageDraw.Draw(grid)
  for phase in range(32):
   p=source/f'target-{target}-phase-{phase}-native-ordered.png';files[str(p)]=sha(p)
   image=Image.open(p).convert('RGB');x=phase%4*256;y=phase//4*276
   grid.paste(image.resize((256,256)),(x,y+20));gd.text((x+5,y+5),f'Native ordered pose {phase}',fill='white')
  p=out/f'target-{target}-all32.png';grid.save(p);files[str(p)]=sha(p)
  comparison=Image.new('RGB',(4*384,2*410),(45,45,45));d=ImageDraw.Draw(comparison)
  p=ROOT/f'state-sign-candidate/native-order-reference-v3/target-{target}-motion.gif';files[str(p)]=sha(p);native=Image.open(p)
  for j,phase in enumerate((0,8,16,24)):
   native.seek(phase);comparison.paste(native.convert('RGB').resize((384,384),Image.Resampling.NEAREST),(j*384,26));d.text((j*384+5,5),f'Native source ordering | pose {phase}',fill='white')
   image=Image.open(source/f'target-{target}-phase-{phase}-native-ordered.png')
   shift=16*math.cos(math.radians(35));scale=512/160;box=(32*scale,(16+shift)*scale,128*scale,(112+shift)*scale)
   comparison.paste(image.transform((384,384),Image.Transform.EXTENT,box,Image.Resampling.BILINEAR).convert('RGB'),(j*384,436));d.text((j*384+5,415),'Private physical sign + scoped source ordering',fill='white')
  p=out/f'target-{target}-source-comparison.png';comparison.save(p);files[str(p)]=sha(p)
 (out/'report.json').write_text(json.dumps({'status':'Presentation, not automatic visual approval','source_report':{'file':str(source/'report.json'),'sha256':sha(source/'report.json')},'files':files,'notes':['Exact native projection top-left.','Controlled ambient phase selected independently; this does not assert synchronized runtime clocks.','Bilinear crop resampling is presentation only.']},indent=2)+'\n')
if __name__=='__main__':main()
