"""Freeze native crown context without assigning shared animation ownership."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement'
OUT=B/'restart2/tree13-canopy-context-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False)
 level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());source=Image.open(B/'baseline/covered.png').convert('RGBA');box=(650,0,1250,240);native=source.crop(box);overlay=native.copy();draw=ImageDraw.Draw(overlay)
 rows=[]
 for index in (8,9,10,11,12,13,14,15,16,75,122):
  m=level['masks'][index];x,y=m['box_top_left'];w,h=m['box_size'];draw.rectangle((x-box[0],y,x+w-box[0]-1,y+h-1),outline=(255,70,180) if index==13 else (60,200,255));draw.text((x-box[0]+1,y+2),str(index),fill='white');rows.append(dict(index=index,origin=[x,y],size=[w,h]))
 sheet=Image.new('RGBA',(1200,520),(40,40,40,255));sheet.paste(native.resize((1200,480)),(0,30));ImageDraw.Draw(sheet).text((8,8),'Original native north woodland; local tree13 lies inside shared crown context',fill='white');sheet.save(OUT/'native-wide.png')
 overlay.resize((1200,480)).save(OUT/'mask-bounds.png')
 m=level['masks'][75];x,y=m['box_top_left'];w,h=m['box_size'];crop=source.crop((x,y,x+w,y+h));crop.putalpha(Image.open(B/'baseline/masks/000075.png').convert('L'));crop.save(OUT/'local75-source.png')
 frames=[]
 for p in sorted((B/'animation-references/animation-05').glob('*.png')):
  im=Image.open(p).convert('RGBA');frames.append(dict(path=str(p.relative_to(ROOT)),sha256=sha(p),size=list(im.size),opaque_pixels=int((np.array(im)[:,:,3]>0).sum())))
 refs=json.loads((B/'restart2/texture-batch-v7/croisement03-tree-25/experiment/foliage-detail-retry-v1/auxiliary-references.json').read_text())['references']
 for r in refs:assert sha(Path(r['parent_image']))==r['parent_sha256']
 proposal=dict(status='PRIVATE construction context; no new source ownership or geometry approval',native_sha256=sha(B/'baseline/covered.png'),bounds=rows,shared_animation_frames=frames,local_static75_sha256=sha(OUT/'local75-source.png'),permitted_references=[dict(asset_id=r['asset_id'],path=r['parent_image'],sha256=r['parent_sha256']) for r in refs],construction_proposal=['Keep all153 accepted bark RGB and lower native stem envelopes exactly.','Replace long offmap needles with branching toward three overlapping irregular crown volumes; use local static75 as contextual footprint, not automatic ownership.','Infer local crown depth at least its width; upper continuation must connect branches within crown, not add unsupported exposed poles.','Full Arbre06 spans multiple neighbouring tree groups and remains a separate animated context; do not duplicate its complete sprite onto tree13.','Before final texture ownership, classify static75 against other native masks and protected153 bark; any unresolved mixed pixels remain reserved.'],limits=['Static75 is a spatial crown candidate, not proven exclusive membership to three stems.','Animated opacity overlap alone does not establish first-hit ownership.'])
 (OUT/'proposal.json').write_text(json.dumps(proposal,indent=2)+'\n')
if __name__=='__main__':main()
