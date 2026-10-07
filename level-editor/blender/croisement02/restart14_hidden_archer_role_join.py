"""Join physical crown samples to exact native layer coordinates without asset edits."""
import hashlib,json
from pathlib import Path
from collections import Counter
from PIL import Image,ImageDraw
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
ROOT=BASE/'restart14-hidden-archer/audit-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 role=ROOT/'source-role-v1/report.json';prov=ROOT/'crown-provenance-v2.json'
 a=json.loads(role.read_text());b=json.loads(prov.read_text());manifest=json.loads((BASE/'restart7-source-patch-delivery/contracts-v1/manifest.json').read_text());resources={r['path']:Path(r['source']) for r in manifest['resources']};cache={};records=[]
 out=ROOT/'source-role-join-v1';out.mkdir(exist_ok=True);assert not (out/'report.json').exists()
 def image(p):
  if p not in cache:cache[p]=Image.open(p).convert('RGBA')
  return cache[p]
 for rec in a['records']:
  samples={tuple(s['pixel']):s for p in b['profiles'] if p['profile']==rec['profile'] for s in p['samples']};ambient=next(l for l in rec['layers'] if l['id']=='animation5');frame=ambient['frame'];ap=resources.get(frame['path'],BASE.parents[1]/'library'/frame['path']);assert sha(ap)==frame['sha256'];im=image(ap);origin=[ambient['position'][i]+frame['offset'][i] for i in range(2)];rows=[]
  for r in rec['samples']:
   x,y=r['pixel'];c=samples[(x,y)];t=c['texture_sample'];packet=Path(t['image']).parent/'partition.json';p=json.loads(packet.read_text());complete=packet.parent/'complete-source.png';src=image(complete);bx,by,_,_=p['native_bbox'];cx,cy=x-bx,y-by;ax,ay=x-origin[0],y-origin[1];rgba=list(im.getpixel((ax,ay))) if 0<=ax<im.width and 0<=ay<im.height else [0,0,0,0];original=list(src.getpixel((cx,cy))) if 0<=cx<src.width and 0<=cy<src.height else [0,0,0,0]
   rows.append({**r,'physical_material':c['material'],'physical_texture':t,'ambient_frame_rgba':rgba,'ambient_frame_texel':[ax,ay],'frozen_crown_at_native_coordinate_rgba':original,'frozen_crown_image':str(complete),'frozen_crown_image_sha256':sha(complete),'partition_sha256':sha(packet),'direct_texture_maps_exact_native_coordinate':Path(t['image']).name=='complete-source.png' and t['texel']==[cx,cy]})
  counts=Counter();
  for r in rows:
   counts['ambient_alpha_zero' if r['ambient_frame_rgba'][3]==0 else 'ambient_alpha_nonzero']+=1
   if r['direct_texture_maps_exact_native_coordinate']:counts['direct_exact_native_coordinate']+=1
   if r['physical_texture']['rgba']==r['native_rgba']:counts['physical_rgb_equals_native']+=1
   if r['physical_texture']['rgba']==r['frozen_crown_at_native_coordinate_rgba']:counts['physical_equals_frozen_crown_coordinate']+=1
  ref=rec['reference'];crop=ref['crop'];native=image(BASE/'restart7-source-patch-delivery/contracts-v1/source-review-v1'/ref['image']);w,h=native.size;panels=[native.copy(),native.copy(),native.copy()]
  for r in rows:
   pos=(r['pixel'][0]-crop[0],r['pixel'][1]-crop[1]);panels[1].putpixel(pos,tuple(r['physical_texture']['rgba']));panels[2].putpixel(pos,tuple(r['ambient_frame_rgba'][:3]+[255]) if r['ambient_frame_rgba'][3] else (255,0,255,255))
  sheet=Image.new('RGB',(w*4*3,h*4+28),(24,24,24));d=ImageDraw.Draw(sheet)
  for i,(panel,label) in enumerate(zip(panels,['Exact native','Replace conflicts with actual crown atlas RGB','Ambient at conflicts; magenta = transparent'])):sheet.paste(panel.convert('RGB').resize((w*4,h*4),Image.Resampling.NEAREST),(i*w*4,28));d.text((i*w*4+3,5),label,fill='white')
  path=out/f'{rec["profile"][-2:]}-{rec["state"]}.png';sheet.save(path);records.append(dict(profile=rec['profile'],state=rec['state'],counts=dict(counts),samples=rows,ambient_frame=ambient,ambient_resource_sha256=sha(ap),image=str(path),image_sha256=sha(path)))
 report=dict(status='Read-only coordinate diagnosis; no geometry, alpha or source ownership changes',inputs={str(role):sha(role),str(prov):sha(prov)},records=records,limits=['A frozen crown source snapshot can differ from the active animated frame; native ordering alone does not authorize deleting physical foliage.','Interior-projected atlas texels do not directly encode native coordinates.','A native-order overlay would be presentation parity only, not arbitrary-view physical occlusion parity.'])
 (out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['profile'],r['state'],r['counts']) for r in records])
if __name__=='__main__':main()
