"""Small immutable-image crops for tree texture review; no model changes."""
from pathlib import Path
from PIL import Image,ImageDraw
import hashlib,json
ROOT=Path(__file__).resolve().parents[3]
e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
out=e/'v5-closeup-diagnostic';out.mkdir(exist_ok=False)
refs=json.loads((e/'auxiliary-references.json').read_text())['references']
items=[('Tree25 native view',e/'baked-preserved-v5/actual/view-0-textured.png',(85,95,185,195)),('Tree25 reverse view 1',e/'baked-preserved-v5/actual/view-1-textured.png',(130,125,230,225)),('Tree25 reverse view 4',e/'baked-preserved-v5/actual/view-4-textured.png',(140,195,240,295)),('Permitted cottage native',Path(refs[0]['parent_image']),(72,62,172,162)),('Permitted moat native',Path(refs[1]['parent_image']),(75,65,175,165)),('Saved generated view 4',e/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png',(140,579,240,679))]
sheet=Image.new('RGB',(1200,860),(32,32,32));draw=ImageDraw.Draw(sheet);receipts=[]
for i,(title,p,box) in enumerate(items):
 im=Image.open(p).convert('RGB');crop=im.crop(box).resize((400,400),Image.Resampling.NEAREST);x=(i%3)*400;y=(i//3)*430;sheet.paste(crop,(x,y+30));draw.text((x+8,y+8),title+' / 4x nearest',fill='white');receipts.append(dict(label=title,path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest(),crop=list(box)))
sheet.save(out/'closeups.png');(out/'images.json').write_text(json.dumps(dict(crops=receipts,note='Equal render-pixel magnification only, not normalized physical leaf scale. Reference native views intentionally exclude unapproved gray patches.'),indent=2)+'\n')
