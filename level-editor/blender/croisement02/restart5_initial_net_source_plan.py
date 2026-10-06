"""Freeze initial rigging source roles without assigning ambiguous high fragments."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';dest=OUT/'restart5-initial-nets/source';dest.mkdir(parents=True,exist_ok=True)
records=[]
for key,anchor,offset,ground_y,rope_x in [('00',[1334,1081],[-45,-143],146,80),('01',[1641,629],[-68,-170],176,122)]:
 path=OUT/f'state-target-evidence/profiles/Trapcr02-{key}/action-0-direction-0-frame-000.png';rgba=np.array(Image.open(path).convert('RGBA'));h,w=rgba.shape[:2];y,x=np.indices((h,w));alpha=rgba[:,:,3]>0
 ground=alpha&(y>=ground_y);rope=alpha&(y>=30)&(y<ground_y)&(x>=rope_x);uncertain=alpha&~ground&~rope
 colors=np.full((h,w,3),40,np.uint8);colors[ground]=[200,140,50];colors[rope]=[60,190,250];colors[uncertain]=[230,60,180]
 Image.fromarray(colors).resize((w*4,h*4),Image.Resampling.NEAREST).save(dest/f'profile-{key}-roles.png')
 for name,mask in [('ground',ground),('rope',rope),('uncertain',uncertain)]:Image.fromarray(mask.astype(np.uint8)*255).save(dest/f'profile-{key}-{name}-mask.png')
 records.append(dict(profile='Trapcr02-'+key,anchor=anchor,offset=offset,source=str(path),source_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),size=[w,h],origin=[anchor[0]+offset[0],anchor[1]+offset[1]],ground_row_start=ground_y,rope_column_start=rope_x,source_roles=dict(ground_net_camouflage=int(ground.sum()),upright_line=int(rope.sum()),ambiguous_native_only=int(uncertain.sum())),roles_image=str(dest/f'profile-{key}-roles.png')))
(dest/'plan.json').write_text(json.dumps(dict(status='Source-supported ground net and upright line; physical depth and support routing inferred',records=records,limitations=['Upper-left sparse pixels have no confirmed physical role; retain native source presentation without turning them into floating solids.','Initial h geometry is separate from triggered e/i bag endpoints.','Ground plane Z0 and original35degree projection; original action/display anchors preserved.','Closed shallow camouflaged mesh and rope thickness infer physical structure; no source pixel reassignment to trees.']),indent=2)+'\n')
