"""CPU inventory of the complete northwest bank and its authored ramp interfaces."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/northwest-bank-source-study-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 O.mkdir(exist_ok=False);level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());src=Image.open(B/'baseline/covered.png').convert('RGB');arrays={}
 for i,rec in enumerate(level['masks']):
  full=np.zeros((src.height,src.width),bool);m=np.array(Image.open(B/f'baseline/masks/{i:06}.png'))>0;x,y=rec['box_top_left'];full[y:y+m.shape[0],x:x+m.shape[1]]=m;arrays[i]=full
 native=arrays[96]|arrays[108]|arrays[109];overlap=[dict(mask=i,layer=level['masks'][i]['layer'],overlap_pixels=int((native&m).sum())) for i,m in arrays.items() if i not in (96,108,109) and (native&m).any()]
 marked=np.array(src);colors={96:(20,210,240),108:(240,80,220),109:(240,180,20)}
 for i,c in colors.items():marked[arrays[i]]=(marked[arrays[i]]*.6+np.array(c)*.4).astype('uint8')
 marked=Image.fromarray(marked);d=ImageDraw.Draw(marked);obstacles=[]
 for i,color in [(52,'white'),(53,'#ff66dd'),(54,'#ffaa22')]:
  ob=level['sight_obstacles'][i];pts=ob['points'];poly=[(p['x'],p['y']-p['z_top']) for p in pts];d.line(poly+[poly[0]],fill=color,width=2);d.text(poly[0],f'{i}',fill=color);obstacles.append(dict(index=i,**ob))
 lip=json.loads((B/'restart2/tree02-ridge-ray-guard-v4/receipt.json').read_text())['ridge_profile'];d.line([(x,y) for x,y,_ in lip],fill='#88ff88',width=2)
 crop=(70,125,780,505);sheet=Image.new('RGB',(1420,420),'#222222');draw=ImageDraw.Draw(sheet)
 for i,(label,im) in enumerate([('Untouched native source',src),('Mask domains + authored top outlines; green = protected shared lip',marked)]):sheet.paste(im.crop(crop),(i*710,30));draw.text((i*710+5,8),label,fill='white')
 sheet.save(O/'source-and-domains.png')
 record=dict(status='CPU source inventory and next-priority proposal; no pure-stone classification or geometry approval',asset='croisement03-northwest-high-rock-outcrop',native_nodes=[52,53,54],catalog_masks=[96,108,109],mask_pixels={str(i):int(arrays[i].sum()) for i in (96,108,109)},union_pixels=int(native.sum()),overlapping_other_masks=overlap,obstacles=obstacles,elevation_interfaces=[e for e in level['elevation_lines'] if e['right_obstacle_index'] in (52,53,54) or e['left_obstacle_index'] in (52,53,54)],native_patch_direct_references=[dict(patch=i,field=k,index=j) for i,p in enumerate(level['patches']) for k in ('old_sight_obstacles','new_sight_obstacles') for j in p[k] if j in (52,53,54)],shared_lip_constraint_sha256=sha(B/'restart2/tree02-ridge-ray-guard-v4/receipt.json'),shared_interface_root_review_sha256=sha(B/'restart2/tree02-shared-ridge-joint-v5/root-shared-interface-review-v1.json'),source_hashes={str(p.relative_to(R)):sha(p) for p in [B/'baseline/Croisement03.rhp.json',B/'baseline/covered.png',R/'level-editor/blender/croisement03/catalog.py']},recommendation='Prioritize full bank52 and ramps53/54 source classification and geometry ahead of Tree01: this removes the provisional rectangular receiver and resolves several reviewed tree root/ledge contexts plus authored elevation joins. Tree01 remains a separate four-component tree geometry obligation.',required_next=['Trace visible stone/earth surfaces against mixed ivy and vegetation; masks are occlusion domains, not exclusive material labels.','Retain exact reviewed x175–340 source lip and source rays; inspect east continuation beneath Trees04–07 before meshing.','Use authored variable-height ramp53/54 and elevation joins, not an arbitrary vertical box across the full plateau.','Separate visible source-facing surface from hidden continuation; terrain ground-color cleanup and any synthesis need their own approved inputs.'],limits=['No new model, render, API or canonical write.','Full native animation/global layering and walkability are not certified by this CPU source survey.','Historical catalog association of masks108/109 is not automatic stone ownership.'])
 (O/'receipt.json').write_text(json.dumps(record,indent=2)+'\n');size=sum(p.stat().st_size for p in O.iterdir());assert size<2*1024**2;print('bytes',size,'maskcounts',record['mask_pixels'],'overlaps',overlap)
if __name__=='__main__':main()
