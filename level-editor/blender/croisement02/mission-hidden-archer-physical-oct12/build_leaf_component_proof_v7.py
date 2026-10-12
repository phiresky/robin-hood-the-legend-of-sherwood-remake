"""Subdivide one source-supported leaf cluster with retained physical patch complement."""
import sys,json,struct,hashlib,io,copy,math
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import label
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q
from operational_records import read_record
ROOT=q.ROOT;DEST=q.OUT/'leaf-component-proof-v7'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def area(poly):return .5*sum(poly[(i+1)%len(poly)][0]*poly[i][1]-poly[(i+1)%len(poly)][1]*poly[i][0] for i in range(len(poly))) if len(poly)>=3 else 0

def clip(poly,axis,limit,keep_greater):
 if len(poly)<3:return []
 out=[]
 for a,b in zip(poly,[*poly[1:],poly[0]]):
  da=(a[axis]-limit)*(1 if keep_greater else -1);db=(b[axis]-limit)*(1 if keep_greater else -1);ain=da>=-1e-9;bin=db>=-1e-9
  if ain:out.append(a)
  if ain!=bin:out.append(a+(b-a)*(da/(da-db)))
 # Drop duplicates at exact cell corners.
 clean=[]
 for p in out:
  if not clean or np.linalg.norm(p-clean[-1])>1e-8:clean.append(p)
 if len(clean)>1 and np.linalg.norm(clean[0]-clean[-1])<1e-8:clean.pop()
 return clean if len(clean)>=3 and abs(area(clean))>1e-8 else []

def split_rect(poly,x,y):
 inside=poly;outside=[]
 for axis,limit,greater in [(0,x,True),(0,x+1,False),(1,y,True),(1,y+1,False)]:
  if not inside:break
  rejected=clip(inside,axis,limit,not greater)
  if rejected:outside.append(rejected)
  inside=clip(inside,axis,limit,greater)
 return inside,outside

def main():
 assert not DEST.exists();DEST.mkdir()
 route=ROOT/'.worktrees/native-editor-delivery/work/active/croisement02/native-editor-operational-packets-v3/static/input-routes.json';src=Path(json.loads(route.read_text())['mission-states/croisement02/reviewed-approved-loops-v1/canopy05/model.glb']);before=sha(src);assert before=='e576f5a00618c5beb74f13bf3a3b485cfd4ce9aa3b0f8fdfc3c59c2f0c8a83e3'
 raw=src.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);binary=raw[28+length:]
 def acc(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']];n={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];assert 'byteStride' not in v;return np.frombuffer(binary,dtype=dtype,count=a['count']*n,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(-1,n)
 mesh=doc['meshes'][doc['nodes'][34]['mesh']];prims=mesh['primitives'];front,back=prims[:2];assert 'observed patches' in doc['materials'][front['material']]['name'];assert 'inferred leaf backs' in doc['materials'][back['material']]['name']
 positions=[acc(p['attributes']['POSITION']) for p in [front,back]];uvs=[acc(p['attributes']['TEXCOORD_0']) for p in [front,back]];triangles=[acc(p['indices']).reshape(-1,3) for p in [front,back]]
 def uvkey(values):return tuple(sorted(tuple(np.round(v*7560).astype(int)) for v in values))
 back_lookup={uvkey(uvs[1][t]):i for i,t in enumerate(triangles[1])}
 native=read_record(q.OUT/'profile02-native-loop-composition-v1/report.json')['loop_frames'][0];image=q.rgba(native);native_rgba=np.array(image);lab,_=label(native_rgba[:,:,3]>0,np.ones((3,3)));yy,xx=np.where(lab==36);assert len(xx)==26;cells=list(zip(xx.tolist(),yy.tolist()));assert [int(xx.min()+140),int(yy.min()),int(xx.max()+140),int(yy.max())]==[502,21,513,27]
 # Continuous inferred envelope avoids interpreting source dithering as physical holes.
 from scipy.spatial import ConvexHull
 corners=np.array([(x+dx,y+dy) for x,y in cells for dx,dy in [(0,0),(1,0),(1,1),(0,1)]],float)
 hull=corners[ConvexHull(corners).vertices]
 def envelope_split(poly):
  inside=poly;outside=[]
  for a,b in zip(hull,np.roll(hull,-1,axis=0)):
   if not inside:break
   edge=b-a
   def half(keep):
    out=[]
    for p,r in zip(inside,inside[1:]+inside[:1]):
     dp=(edge[0]*(p[1]-a[1])-edge[1]*(p[0]-a[0]))*keep;dr=(edge[0]*(r[1]-a[1])-edge[1]*(r[0]-a[0]))*keep
     if dp>=-1e-9:out.append(p)
     if (dp>=-1e-9)!=(dr>=-1e-9):out.append(p+(r-p)*(dp/(dp-dr)))
    clean=[]
    for p in out:
     if not clean or np.linalg.norm(p-clean[-1])>1e-7:clean.append(p)
    if len(clean)>1 and np.linalg.norm(clean[0]-clean[-1])<1e-7:clean.pop()
    return clean if len(clean)>=3 and abs(area(clean))>1e-7 else []
   rejected=half(-1)
   if rejected:outside.append(rejected)
   inside=half(1)
  return inside,outside
 # World-transform flattening retains exactly the original glTF scene coordinates.
 parents={child:i for i,n in enumerate(doc['nodes']) for child in n.get('children',[])}
 def world(i):return (world(parents[i]) if i in parents else np.eye(4))@q.matrix(doc['nodes'][i])
 W=world(34);linear=W[:3,:3]
 storage={(owner,side):dict(pos=[],uv=[],normal=[],color=[],pairpos=[],paircolor=[],pairmorph=[[] for _ in front.get('targets',[])],morph=[[] for _ in front.get('targets',[])]) for owner in [0,1] for side in [0,1]};records=[];max_error=0
 def emit(owner,side,poly,ti,bti):
  ids=triangles[side][ti if side==0 else bti];uv=uvs[side][ids]*[756,570];M=np.vstack([uv.T,np.ones(3)]);inv=np.linalg.inv(M);P=positions[side][ids]
  # Match source primitive winding independently for each side.
  if np.sign(area(list(uv)))!=np.sign(area(poly)):poly=list(reversed(poly))
  target=storage[(owner,side)]
  for j in range(1,len(poly)-1):
   ptri=[poly[0],poly[j],poly[j+1]];bary=np.array([inv@np.r_[p,1] for p in ptri]);local=bary@P;glob=(np.c_[local,np.ones(3)]@W.T)[:,:3];normal=np.cross(glob[1]-glob[0],glob[2]-glob[0]);normal/=np.linalg.norm(normal)
   color_index=(front if side==0 else back)['attributes'].get('COLOR_0');colors=np.ones((3,4)) if color_index is None else acc(color_index)[ids].astype(float)
   if color_index is not None and doc['accessors'][color_index].get('normalized'):colors/=255 if doc['accessors'][color_index]['componentType']==5121 else 65535
   target['color'].extend((bary@colors).tolist())
   if side==0:
    bids=triangles[1][bti];binv=np.linalg.inv(np.vstack([(uvs[1][bids]*[756,570]).T,np.ones(3)]));bbary=np.array([binv@np.r_[p,1] for p in ptri]);paired=(np.c_[bbary@positions[1][bids],np.ones(3)]@W.T)[:,:3];target['pairpos'].extend(paired.tolist())
    ci=back['attributes'].get('COLOR_0');bc=np.ones((3,4)) if ci is None else acc(ci)[bids].astype(float)
    if ci is not None and doc['accessors'][ci].get('normalized'):bc/=255 if doc['accessors'][ci]['componentType']==5121 else 65535
    target['paircolor'].extend((bbary@bc).tolist())
    for k,morph in enumerate(back.get('targets',[])):target['pairmorph'][k].extend(((bbary@acc(morph['POSITION'])[bids])@linear.T).tolist())
   target['pos'].extend(glob.tolist());target['uv'].extend((np.array(ptri)/[756,570]).tolist());target['normal'].extend([normal.tolist()]*3)
   for k,morph in enumerate((front if side==0 else back).get('targets',[])):
    delta=acc(morph['POSITION'])[ids];target['morph'][k].extend(((bary@delta)@linear.T).tolist())
 for ti,t in enumerate(triangles[0]):
  uv=uvs[0][t]*[756,570]
  if uv[:,0].max()<xx.min() or uv[:,0].min()>xx.max()+1 or uv[:,1].max()<yy.min() or uv[:,1].min()>yy.max()+1:continue
  if abs(area(list(uv)))<1e-8:continue
  bti=back_lookup.get(uvkey(uvs[0][t]));assert bti is not None,ti
  inside,remainder=envelope_split(list(uv));leaf=[inside] if inside else []
  if not leaf:continue
  before_area=abs(area(list(uv)));after_area=sum(abs(area(p)) for p in leaf+remainder);error=abs(before_area-after_area);max_error=max(max_error,error);assert error<1e-6
  for owner,polys in [(0,leaf),(1,remainder)]:
   for poly in polys:
    emit(owner,0,poly,ti,bti);emit(owner,1,poly,ti,bti)
  records.append(dict(front_triangle=ti,back_triangle=bti,source_area_pixels=before_area,leaf_area_pixels=sum(abs(area(p)) for p in leaf),complement_area_pixels=sum(abs(area(p)) for p in remainder)))
 assert records
 # Conform all edge subdivisions in paired physical coordinates, never projected UV.
 # Pairing includes both inherited surfaces so overlapping source pixels remain distinct.
 def conform(data):
  pair=np.c_[data['pos'],data['pairpos']]
  # Collapse sub-float32 seam noise in paired coordinates, never across layers.
  from scipy.spatial import cKDTree
  parent=list(range(len(pair)))
  def find(i):
   while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
   return i
  for a,b in cKDTree(pair).query_pairs(0.0005):
   a,b=find(a),find(b)
   if a!=b:parent[max(a,b)]=min(a,b)
  mapping=[find(i) for i in range(len(pair))]
  for field in ['pos','pairpos','paircolor','uv','normal','color']:data[field]=[data[field][i] for i in mapping]
  for field in ['morph','pairmorph']:data[field]=[[target[i] for i in mapping] for target in data[field]]
  pair=np.c_[data['pos'],data['pairpos']];unique={}
  for i,p in enumerate(pair):unique.setdefault(tuple(np.round(p,7)),i)
  values=pair[list(unique.values())];out={k:[] for k in ['pos','pairpos','paircolor','uv','normal','color']};out['morph']=[[] for _ in data['morph']];out['pairmorph']=[[] for _ in data['pairmorph']]
  for off in range(0,len(pair),3):
   tri=pair[off:off+3];boundary=[]
   for ia,ib in [(0,1),(1,2),(2,0)]:
    a,b=tri[ia],tri[ib];delta=b-a;den=float(delta@delta)
    if den<1e-18:continue
    t=(values-a)@delta/den;dist=np.linalg.norm(values-(a+t[:,None]*delta),axis=1);ts=sorted(set([0.0]+[float(x) for x in t[(dist<1e-6)&(t>1e-7)&(t<1-1e-7)]]))
    for f in ts:
     weights=np.zeros(3);weights[ia]=1-f;weights[ib]=f
     if not boundary or np.linalg.norm(weights@tri-boundary[-1]@tri)>1e-7:boundary.append(weights)
   if len(boundary)<3 or np.linalg.norm(np.cross(tri[1,:3]-tri[0,:3],tri[2,:3]-tri[0,:3]))<1e-10:continue
   center=np.mean(boundary,axis=0)
   for a,b in zip(boundary,boundary[1:]+boundary[:1]):
    bary=np.array([center,a,b]);positions=bary@np.array(data['pos'][off:off+3]);backs=bary@np.array(data['pairpos'][off:off+3])
    # Exact float32 collapse is removed before side generation; retain real wedges.
    fa=np.cross(*(positions.astype('f4')[1:]-positions.astype('f4')[0]));ba=np.cross(*(backs.astype('f4')[1:]-backs.astype('f4')[0]))
    if np.linalg.norm(fa)==0 and np.linalg.norm(ba)==0:continue
    for k in ['pos','pairpos','paircolor','uv','normal','color']:out[k].extend((bary@np.array(data[k][off:off+3])).tolist())
    for k in ['morph','pairmorph']:
     for j,m in enumerate(data[k]):out[k][j].extend((bary@np.array(m[off:off+3])).tolist())
  return out
 fp=conform(storage[(0,0)]);storage[(0,0)]=fp
 # Generate the back from the paired records, with opposite winding.
 bp=dict(pos=[],uv=[],normal=[],color=[],morph=[[] for _ in fp['morph']])
 for off in range(0,len(fp['pos']),3):
  ids=[off,off+2,off+1]
  for k,field in [('pos','pairpos'),('uv','uv'),('color','paircolor')]:bp[k].extend([fp[field][i] for i in ids])
  ps=np.array([fp['pairpos'][i] for i in ids]);normal=np.cross(ps[1]-ps[0],ps[2]-ps[0]);nn=np.linalg.norm(normal);bp['normal'].extend([(normal/nn if nn else np.zeros(3)).tolist()]*3)
  for j,m in enumerate(fp['pairmorph']):bp['morph'][j].extend([m[i] for i in ids])
 storage[(0,1)]=bp
 edge_map={}
 def paired_key(i):return tuple(np.round(np.r_[fp['pos'][i],fp['pairpos'][i]],6))
 for k in range(0,len(fp['pos']),3):
  for a,b in [(k,k+1),(k+1,k+2),(k+2,k)]:
   key=tuple(sorted([paired_key(a),paired_key(b)]));edge_map.setdefault(key,[]).append((a,b))
 walls=dict(pos=[],uv=[],normal=[],color=[],morph=[[] for _ in fp['morph']])
 for edges in edge_map.values():
  if len(edges)!=1:continue
  a,b=edges[0];vertices=[fp['pos'][a],fp['pos'][b],fp['pairpos'][b],fp['pairpos'][a]];uv=[fp['uv'][a],fp['uv'][b],fp['uv'][b],fp['uv'][a]]
  for inds in [(0,2,1),(0,3,2)]:
   ps=np.array([vertices[i] for i in inds]);normal=np.cross(ps[1]-ps[0],ps[2]-ps[0]);norm=np.linalg.norm(normal)
   if norm<1e-12:continue
   walls['color'].extend([fp['paircolor'][a]]*3);walls['pos'].extend(ps.tolist());walls['uv'].extend([uv[i] for i in inds]);walls['normal'].extend([(normal/norm).tolist()]*3)
   for j in range(len(walls['morph'])):
    vals=[fp['morph'][j][a],fp['morph'][j][b],fp['pairmorph'][j][b],fp['pairmorph'][j][a]];walls['morph'][j].extend([vals[i] for i in inds])
 # Standalone focused patch proof; full reusable tree remains immutable referenced parent.
 out=dict(asset=dict(version='2.0',generator='source-owned physical leaf subdivision proof'),scene=0,scenes=[dict(nodes=[0,1])],nodes=[],meshes=[],materials=[],textures=[],images=[],samplers=doc.get('samplers',[]),buffers=[dict(byteLength=0)],bufferViews=[],accessors=[]);chunks=bytearray()
 def blob(data):
  while len(chunks)%4:chunks.append(0)
  offset=len(chunks);chunks.extend(data);out['bufferViews'].append(dict(buffer=0,byteOffset=offset,byteLength=len(data)));return len(out['bufferViews'])-1
 def array(values,kind):
  data=np.array(values,dtype='<f4');view=blob(data.tobytes());a=dict(bufferView=view,componentType=5126,count=len(data),type=kind)
  if kind=='VEC3':a.update(min=data.min(0).tolist(),max=data.max(0).tolist())
  out['accessors'].append(a);return len(out['accessors'])-1
 def import_material(index):
  mat=copy.deepcopy(doc['materials'][index]);tex=doc['textures'][mat['pbrMetallicRoughness']['baseColorTexture']['index']];img=doc['images'][tex['source']];v=doc['bufferViews'][img['bufferView']];image_index=len(out['images']);out['images'].append(dict(bufferView=blob(binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]),mimeType=img['mimeType']));newtex=copy.deepcopy(tex);newtex['source']=image_index;out['textures'].append(newtex);mat['pbrMetallicRoughness']['baseColorTexture']['index']=len(out['textures'])-1
  # Retain only the documented color texture; these source materials have no other maps.
  assert not any(k in mat for k in ['normalTexture','occlusionTexture'])
  if 'emissiveTexture' in mat:
   assert mat['emissiveTexture']['index']==doc['materials'][index]['pbrMetallicRoughness']['baseColorTexture']['index'];mat['emissiveTexture']['index']=len(out['textures'])-1
  out['materials'].append(mat);return len(out['materials'])-1
 frontmat=import_material(front['material']);backmat=import_material(back['material']);native_atlas=Image.new('RGBA',(756,570));native_atlas.alpha_composite(image,(native['top_left'][0]-140,native['top_left'][1]));stream=io.BytesIO();native_atlas.save(stream,format='PNG');out['images'].append(dict(bufferView=blob(stream.getvalue()),mimeType='image/png'));out['samplers'].append(dict(magFilter=9728,minFilter=9728,wrapS=33071,wrapT=33071));out['textures'].append(dict(source=len(out['images'])-1,sampler=len(out['samplers'])-1));mat=copy.deepcopy(out['materials'][frontmat]);mat['name']='Native frame0 leaf-cluster36 exact known front';mat['emissiveTexture']['index']=len(out['textures'])-1;mat['pbrMetallicRoughness']['baseColorTexture']['index']=len(out['textures'])-1;out['materials'].append(mat);nativemat=len(out['materials'])-1
 def primitive(data,material):
  p=dict(attributes=dict(POSITION=array(data['pos'],'VEC3'),NORMAL=array(data['normal'],'VEC3'),TEXCOORD_0=array(data['uv'],'VEC2'),COLOR_0=array(data['color'],'VEC4')),material=material,mode=4)
  if data['morph']:p['targets']=[dict(POSITION=array(m,'VEC3')) for m in data['morph']]
  return p
 for owner in [0,1]:
  pp=[primitive(storage[(owner,0)],nativemat if owner==0 else frontmat),primitive(storage[(owner,1)],backmat)]
  if owner==0 and walls['pos']:pp.append(primitive(walls,backmat))
  out['meshes'].append(dict(primitives=pp,weights=mesh.get('weights',[0]*len(front.get('targets',[])))))
  out['nodes'].append(dict(name='Native leaf cluster36 physical component' if owner==0 else 'Retained local static crown complement',mesh=owner,extras=dict(source_role='source-owner' if owner==0 else 'static-receiver',source_model_sha256=before,source_mesh_node=34,scope='LOCAL_PATCH_PROOF_NOT_COMPLETE_TREE_EXPORT')))
 # Preserve the original crown morph timing on both patch components.
 animations=[]
 for anim in doc.get('animations',[]):
  chans=[ch for ch in anim['channels'] if ch['target']['node']==34];assert all(ch['target']['path']=='weights' for ch in chans)
  if not chans:continue
  aa=dict(name=anim['name'],samplers=[],channels=[])
  for ch in chans:
   s=copy.deepcopy(anim['samplers'][ch['sampler']]);s['input']=array(acc(s['input']),'SCALAR');s['output']=array(acc(s['output']),'SCALAR');aa['samplers'].append(s)
   for owner in [0,1]:aa['channels'].append(dict(sampler=len(aa['samplers'])-1,target=dict(node=owner,path='weights')))
  animations.append(aa)
 out['animations']=animations
 while len(chunks)%4:chunks.append(0)
 out['buffers'][0]['byteLength']=len(chunks);j=json.dumps(out,separators=(',',':')).encode();j+=b' '*((-len(j))%4);total=12+8+len(j)+8+len(chunks);model=DEST/'component-proof.glb';model.write_bytes(struct.pack('<III',0x46546c67,2,total)+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(chunks),0x004e4942)+chunks)
 roles=dict(version=1,model_sha256=sha(model),owner=dict(kind='map-animation',index=5),components=[dict(node=0,role='source-owner'),dict(node=1,role='static-receiver')]);(DEST/'components.json').write_text(json.dumps(roles,indent=2)+'\n')
 assert sha(src)==before
 report=dict(status='CONTINUOUS_ENVELOPE_REQUIRES_SAVED_TOPOLOGY_AND_SOURCE_CAPTURE',source_model=dict(path=str(src),sha256=before,unchanged=True),candidate=dict(path=str(model),sha256=sha(model),bytes=model.stat().st_size),native_source=native,cluster=dict(id=36,pixels=26,source_bbox=[502,21,514,28],physical_envelope='inferred convex hull of native pixel cells',envelope_uv_pixels=hull.tolist(),envelope_area=abs(area(list(hull)))),triangle_records=records,source_area_partition_max_error=max_error,leaf_front_triangles=len(fp['pos'])//3,leaf_back_triangles=len(bp['pos'])//3,leaf_side_triangles=len(walls['pos'])//3,static_complement_front_triangles=len(storage[(1,0)]['pos'])//3,morph_targets=len(front.get('targets',[])),limits=['Bounded local patch proof only; original complete reusable tree unchanged and retained as parent.','Known animated component front uses native loopframe0 source pixels; static complement retains original source-front material/UV.','Existing paired front/back surfaces and morph deltas barycentrically subdivided; paired physical boundary walls constructed; closure requires saved topology audit.','No sample alpha trimming or whole-crown role reassignment. Actual source-camera color/depth proof still required before scaling or review approval.'])
 (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
