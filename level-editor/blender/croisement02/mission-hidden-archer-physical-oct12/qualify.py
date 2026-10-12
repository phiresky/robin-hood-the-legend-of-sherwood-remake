"""Source-camera bounds and state evidence for four independently scoped foliage screens."""
import hashlib,itertools,json,math,struct
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[4]
LIB=ROOT/'level-editor/library'
OUT=ROOT/'level-editor/work/croisement02-refinement/mission-hidden-archer-physical-oct12-v1'
BIND=ROOT/'level-editor/work/croisement02-refinement/mission-coupling-oct12-v1/bindings-v2.json'
def pin(p):return dict(path=str(p.relative_to(ROOT)),sha256=hashlib.sha256(p.read_bytes()).hexdigest())
def matrix(n):
 if 'matrix'in n:return np.array(n['matrix']).reshape(4,4).T
 x,y,z,w=n.get('rotation',[0,0,0,1]);m=np.eye(4);m[:3,:3]=np.array([[1-2*y*y-2*z*z,2*x*y-2*z*w,2*x*z+2*y*w],[2*x*y+2*z*w,1-2*x*x-2*z*z,2*y*z-2*x*w],[2*x*z-2*y*w,2*y*z+2*x*w,1-2*x*x-2*y*y]])@np.diag(n.get('scale',[1,1,1]));m[:3,3]=n.get('translation',[0,0,0]);return m

def bounds(source,placement):
 path=LIB/source['model']
 with path.open('rb') as f:
  h=f.read(20);size=struct.unpack_from('<I',h,12)[0];doc=json.loads(f.read(size))
 points=[]
 def walk(i,parent):
  n=doc['nodes'][i];mat=parent@matrix(n)
  if 'mesh'in n:
   for p in doc['meshes'][n['mesh']]['primitives']:
    a=doc['accessors'][p['attributes']['POSITION']];assert 'min'in a and 'max'in a
    v=np.array(list(itertools.product(*zip(a['min'],a['max']))));points.extend((np.c_[v,np.ones(len(v))]@mat.T)[:,:3])
  for child in n.get('children',[]):walk(child,mat)
 for i in doc['scenes'][doc.get('scene',0)]['nodes']:walk(i,np.eye(4))
 v=np.array(points)
 if not len(v):return dict(id=source['id'],empty_scene=True)
 # GLTF Y-up -> editor scene Z-up, then the authored placement.
 v=v[:,[0,2,1]]*np.array([1,-1,1]);t=placement['transform'];assert t['rot_deg']==0
 s,c=math.sin(math.radians(35)),math.cos(math.radians(35));v+=np.array([t['dx'],-t['dy']/s,t['dz']]);uv=np.c_[v[:,0],-v[:,1]*s-v[:,2]*c]
 return dict(id=source['id'],model=source['model'],model_sha256=source['model_sha256'],descriptor=source['descriptor'],descriptor_sha256=source['descriptor_sha256'],placement=placement,projected_aabb=[*uv.min(0),*uv.max(0)],world_aabb=[v.min(0).tolist(),v.max(0).tolist()])
def rgba(frame):
 a=np.array(Image.open(ROOT/frame['image']['path']).convert('RGBA'));rgb=a[:,:,:3].astype(np.uint16);key=(rgb[:,:,0]>>3)<<11|(rgb[:,:,1]>>2)<<5|(rgb[:,:,2]>>3);a[:,:,3][(key==0x07c0)|(key==31)]=0;return Image.fromarray(a)
def main():
 OUT.mkdir(exist_ok=True,parents=True);mp=LIB/'scenes/croisement02.rhlos-map.json';m=json.loads(mp.read_text());sources={r['id']:r for r in m['assetSources']};models=[bounds(sources[a],p) for p in m['placements'] for a in p['assets'] if a in sources]
 b=json.loads(BIND.read_text());unique={r['profile']:r for r in b['rows'] if 'hidden archer'in r['profile']};bgpath=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Day/Croisement02.map.png';bg=Image.open(bgpath).convert('RGBA');rows=[]
 for number,(name,r) in enumerate(sorted(unique.items()),1):
  frames=r['art']['states']['initial']['frames']+r['art']['states']['transition']['frames'];box=[int(min(f['top_left'][i] for f in frames)) for i in range(2)]+[int(max(f['top_left'][i]+f['size'][i] for f in frames)) for i in range(2)];crop=[box[0]-45,box[1]-45,box[2]+45,box[3]+45];w,h=crop[2]-crop[0],crop[3]-crop[1];sheet=Image.new('RGB',(w*5,h+25),(35,35,35));d=ImageDraw.Draw(sheet)
  states=[]
  for i,f in enumerate([None]+frames):
   im=bg.crop(crop)
   if f:im.alpha_composite(rgba(f),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]))
   sheet.paste(im,(i*w,25));d.text((i*w+3,5),['Base artwork','Initial','Transition 0','Transition 1','Integrated terminal'][i],fill='white')
   if f:
    mask=Image.new('L',(w,h));a=rgba(f);mask.paste(a.getchannel('A'),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]));states.append(np.array(mask)>0)
  target=OUT/f'archer{number:02}-source-context.png';sheet.save(target)
  candidates=[x for x in models if not x.get('empty_scene') and x['projected_aabb'][0]<box[2] and x['projected_aabb'][2]>box[0] and x['projected_aabb'][1]<box[3] and x['projected_aabb'][3]>box[1]]
  for x in candidates:
   assert pin(LIB/x['model'])['sha256']==x['model_sha256'];assert pin(LIB/x['descriptor'])['sha256']==x['descriptor_sha256']
  rows.append(dict(profile=name,occurrences=[dict(mission=x['mission'],patch_index=x['patch_index']) for x in b['rows'] if x['profile']==name],art=r['art'],source_union_box=box,source_context=pin(target),initial_to_terminal=dict(initial_pixels=int(states[0].sum()),terminal_pixels=int(states[-1].sum()),removed_pixels=int((states[0]&~states[-1]).sum()),added_pixels=int((states[-1]&~states[0]).sum())),bounds_candidates=candidates))
 report=dict(status='PRIVATE_SOURCE_AND_PROJECTED_BOUNDS_ONLY',map=pin(mp),bindings=pin(BIND),background=pin(bgpath),rows=rows,limitations=['Projected accessor boxes are conservative broad phase, not triangle visibility or physical contact proof.','No geometry change, approval inheritance, rendering, synthesis or canonical installation performed.','Initial and three native transition images are independently pinned; terminal is the final transition frame integrated into background.'])
 (OUT/'qualification.json').write_text(json.dumps(report,indent=2)+'\n');print([(x['profile'],[a['id'] for a in x['bounds_candidates']]) for x in rows])
if __name__=='__main__':main()
