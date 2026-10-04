"""Controlled sign/foliage draw-order reference with distinct display and action points."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def behind(poly,point):
 x,y=point
 if x<poly[0][0]:return y<poly[0][1]
 if x>poly[-1][0]:return y<poly[-1][1]
 for a,b in zip(poly,poly[1:]):
  if b[0]>=x:return (b[0]-a[0])*(y-a[1])-(b[1]-a[1])*(x-a[0])<0
 raise ValueError('Unordered display polyline')

def placed_frame(row,box):
 x,y,w,h=row['bbox'];im=Image.new('RGBA',(box[2]-box[0],box[3]-box[1]));im.alpha_composite(Image.open(row['image']).convert('RGBA'),(x-box[0],y-box[1]));return im

def main():
 root=OUT/'state-sign-candidate';dst=root/'native-order-reference-v3';dst.mkdir(exist_ok=False);manifest=OUT/'state-target-evidence/manifest.json';alltargets=json.loads(manifest.read_text());instances=[r for r in alltargets['instances']if r['profile_id']=='TG_Panel-12'];profile=next(r for r in alltargets['profiles']if r['id']=='TG_Panel-12');frames=next(r for r in profile['rows']if r['action_id']==0)['frames'];animation_path=OUT/'animation-references/manifest.json';animations=json.loads(animation_path.read_text())['animations'];poly_animations=[dict(kind='map-animation',index=r['index'],poly=r['display_polyline'],key=min(p[1]for p in r['display_polyline']))for r in animations if r['display_polyline']];poly_targets=[dict(kind='mission-target',index=r['target_index'],poly=r['target']['polyline'],key=min(p[1]for p in r['target']['polyline']))for r in alltargets['instances']if r['mission']=='S03_FoB_MP'and r['target']['polyline']];ordered=sorted(poly_animations+poly_targets,key=lambda r:r['key']);rows=[];sheet=Image.new('RGB',(5*256,256),(80,80,80))
 for n,instance in enumerate(instances):
  t=instance['target'];x,y=t['position_x'],t['position_y'];box=(x-48,y-64,x+48,y+32);base=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop(box);action_point=[t['action_position_x'],t['action_position_y']]
  if t['polyline']:insertion=dict(kind='mission-target',index=instance['target_index'],key=min(p[1]for p in t['polyline']))
  else:
   trigger=next((r for r in ordered if behind(r['poly'],action_point)),None);insertion=dict(kind='before-first-behind-polyline',trigger=trigger,key=trigger['key']-.001 if trigger else float('inf'))
  sign_order_y=y+float(t['position_z'] if t['position_z']>=0 else (36.000907 if t['obstacle_index']==0 else 0))
  sign_rank=(insertion['key'],sign_order_y)
  relevant=[]
  for a in animations:
   overlap=any(f['bbox'][0]<box[2]and f['bbox'][0]+f['bbox'][2]>box[0]and f['bbox'][1]<box[3]and f['bbox'][1]+f['bbox'][3]>box[1]for f in a['frames'])
   if not overlap:continue
   if a['display_polyline']:rank=(min(p[1]for p in a['display_polyline']),0)
   else:
    sprite=a['sprite']; native_manifest=Path(a['frames'][0]['source']).parents[2]/'manifest.json';native_profile=next(p for p in json.loads(native_manifest.read_text())['profiles']if p['name']==sprite['profile_name']);point=[sprite['position_x']+native_profile['center_x'],sprite['position_y']+native_profile['center_y']];trigger=next((r for r in ordered if behind(r['poly'],point)),None);rank=(trigger['key']-.001 if trigger else float('inf'),point[1]+sprite['elevation'])
   relevant.append((rank,a))
  assert len(relevant)<=1,'Multiple overlay cycles require joint phase enumeration'
  phase_count=max([len(a['frames'])for _,a in relevant],default=1);proof=[];motion=[]
  for i,f in enumerate(frames):
   target=Image.new('RGBA',base.size);target.alpha_composite(Image.open(f['image']).convert('RGBA'),(x+int(f['offset'][0])-box[0],y+int(f['offset'][1])-box[1]));raw=np.asarray(target)[:,:,3]>127
   for phase in range(phase_count):
    canvas=base.copy();foreground=np.zeros_like(raw)
    for key,a in sorted(relevant,key=lambda z:z[0]):
     if key<sign_rank:canvas.alpha_composite(placed_frame(a['frames'][phase%len(a['frames'])],box))
    canvas.alpha_composite(target)
    for key,a in sorted(relevant,key=lambda z:z[0]):
     if key>sign_rank:
      overlay=placed_frame(a['frames'][phase%len(a['frames'])],box);foreground|=np.asarray(overlay)[:,:,3]>127;canvas.alpha_composite(overlay)
    proof.append(dict(sign_pose=i,overlay_phase=phase,raw_target_pixels=int(raw.sum()),covered_by_later_overlay=int((raw&foreground).sum()),visible_target_pixels=int((raw&~foreground).sum())))
    if phase==(i//2)%phase_count:motion.append(canvas.convert('RGB').resize((288,288),Image.Resampling.NEAREST))
    if i==phase==0:
     canvas.save(dst/f'target-{instance["target_index"]}-phase0.png');Image.fromarray((raw&~foreground).astype('uint8')*255).save(dst/f'target-{instance["target_index"]}-visible-phase0.png');sheet.paste(canvas.convert('RGB').resize((256,256),Image.Resampling.NEAREST),(n*256,0))
   assert len(motion)==i+1
  motion[0].save(dst/f'target-{instance["target_index"]}-motion.gif',save_all=True,append_images=motion[1:],duration=80,loop=0)
  rows.append(dict(target_index=instance['target_index'],display_position=[x,y],action_position=action_point,insertion=insertion,overlapping_animations=[dict(index=a['index'],profile=a['profile'],key=key,after_sign=key>sign_rank)for key,a in relevant],phase_results=proof))
 sheet.save(dst/'five-native-phase0.png');write_json(dst/'manifest.json',dict(status='Controlled source reference, not a gameplay screenshot or 3D integration approval',target_manifest_sha256=sha(manifest),animation_manifest_sha256=sha(animation_path),records=rows,semantics=['Empty-polyline targets merge with non-animations using their action map point, after their physical/display position was established separately.','Polyline targets sort by minimum polylineY; every global polyline is considered before determining insertion, including nonoverlapping ones.','Visible source alpha uses later animation RGBA, not gameplay occupancy masks. Static background lies beneath the target draw.'],limitations=['Mission target active-state variations do not alter these insertion outcomes: the first applicable map polyline precedes target-trap entries.','Controlled relative canopy phases are enumerated; wall-clock phase is independent.','Other moving actors are absent. Butterfly source frames are included where their path enters the crop; their native profile center and elevation establish non-animation ordering.','This is a source-order reference; full physical scene/canopy appearance validation remains separate.']))
 print([(r['target_index'],r['overlapping_animations'],r['phase_results'][0])for r in rows])

if __name__=='__main__':main()
