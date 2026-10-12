"""Partition current receiver triangles into reversible hole caps without repainting."""
import sys,json,copy,math,struct,hashlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
import lossy_assets as la
BASE=ROOT/'level-editor/work/active/croisement02';LIB=ROOT/'level-editor/library';DEST=BASE/'restart39-current-hole-apertures-v1';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return dict(path=str(p),sha256=sha(p))
def read(p):return json.loads(Path(p).read_text())
def area(p):
 a=np.asarray(p);return abs(float(np.sum(a[:,0]*np.roll(a[:,1],-1)-a[:,1]*np.roll(a[:,0],-1))))*.5 if len(a)>2 else 0.
def half(poly,a,b,inside):
 if not poly:return []
 out=[];a=np.array(a);edge=np.array(b)-a
 def f(v):q=np.array(v)-a;return edge[0]*q[1]-edge[1]*q[0]
 prior=poly[-1];pv=f(prior)
 for now in poly:
  nv=f(now);pi=pv>=-1e-10 if inside else pv<=1e-10;ni=nv>=-1e-10 if inside else nv<=1e-10
  if pi!=ni:
   t=pv/(pv-nv);out.append((np.array(prior)+(np.array(now)-prior)*t).tolist())
  if ni:out.append(now)
  prior=now;pv=nv
 return out
def cut(poly,ellipse):
 current=poly;outside=[]
 for a,b in zip(ellipse,ellipse[1:]+ellipse[:1]):
  other=half(current,a,b,False)
  if area(other)>1e-9:outside.append(other)
  current=half(current,a,b,True)
  if area(current)<1e-9:return outside,[]
 return outside,current

def partition(asset,source,placement,holes):
 path=LIB/source['model'];assert sha(path)==source['model_sha256'];doc,buffers,_=la.read_glb(path);assert len(buffers)==1;assert not doc.get('animations')and not doc.get('skins');original=copy.deepcopy(doc);binary=bytearray(buffers[0]);original_binary_sha=hashlib.sha256(binary).hexdigest();offset=np.array([placement['dx'],-placement['dy']/SIN,placement['dz']]);assert placement['rot_deg']==0
 # The standalone bank wrapper is discarded by the production placement loader.
 roots=doc['scenes'][doc.get('scene',0)]['nodes'];assert len(roots)==1
 if doc['nodes'][roots[0]].get('name')=='map':roots=doc['nodes'][roots[0]]['children']
 root_ids=list(roots);base_nodes=len(doc['nodes']);records=[];cap_areas={h['id']:0. for h in holes};all_provenance={};guards=[]
 def accessor(values,kind):
  nonlocal binary
  arr=np.asarray(values,dtype='<u4'if kind=='SCALAR'else'<f4');binary.extend(b' '*(-len(binary)%4));start=len(binary);data=arr.tobytes();binary.extend(data);view=len(doc['bufferViews']);doc['bufferViews'].append(dict(buffer=0,byteOffset=start,byteLength=len(data)));i=len(doc['accessors']);r=dict(bufferView=view,componentType=5125 if kind=='SCALAR'else 5126,count=len(arr),type=kind)
  if kind=='VEC3':r.update(min=arr.min(axis=0).astype(float).tolist(),max=arr.max(axis=0).astype(float).tolist())
  doc['accessors'].append(r);return i
 def clone(index,matrix):
  n=copy.deepcopy(original['nodes'][index]);matrix=matrix@la.node_matrix(n);assert np.allclose(matrix[:3,:3],np.eye(3),atol=1e-12),'Unexpected unbaked source transform';new=len(doc['nodes']);doc['nodes'].append(n);n['children']=[clone(c,matrix)for c in n.get('children',[])]
  if 'mesh'not in n:return new
  mesh=original['meshes'][n['mesh']];groups={};total_src=total_dst=0
  for pi,p in enumerate(mesh['primitives']):
   assert p.get('mode',4)==4 and not p.get('targets');ids=la.accessor_array(original,buffers,p['indices']).reshape(-1,3);attrs={k:la.accessor_array(original,buffers,v,dequantize=True)for k,v in p['attributes'].items()};assert not any(k.startswith(('JOINTS','WEIGHTS'))for k in attrs);xyz=attrs['POSITION'];local=np.column_stack((xyz,np.ones(len(xyz))));world=(local@matrix.T)[:,:3]+offset;screen=np.column_stack((world[:,0],-world[:,1]*SIN-world[:,2]*COS));batches={};proof=[];maxerr=0.;splits=0
   for ti,inds in enumerate(ids):
    coords=screen[inds];poly=coords.tolist();a0=area(poly);hits=[]
    for h in holes:
     if np.max(np.abs(world[inds,2]-h['support_z']))>.02:continue
     ellipse=np.array(h['ellipse'])
     if np.all(coords.max(0)>=ellipse.min(0))and np.all(coords.min(0)<=ellipse.max(0))and a0>1e-8:
      _,inner=cut(poly,h['ellipse'])
      if inner:hits.append(h)
    if not hits:
     batches.setdefault('outside',[]).append((ti,np.eye(3)));continue
    splits+=1;inv=np.linalg.inv(np.vstack((coords.T,np.ones(3))));parts=[('outside',poly)]
    for h in hits:
     more=[]
     for tag,q in parts:
      if tag!='outside':more.append((tag,q));continue
      outer,inner=cut(q,h['ellipse']);more.extend(('outside',v)for v in outer)
      if inner:more.append((h['id'],inner))
     parts=more
    err=abs(sum(area(q)for _,q in parts)-a0);maxerr=max(maxerr,err);assert err<max(1e-4,a0*1e-9),(asset,ti,err)
    for tag,q in parts:
     if tag!='outside':cap_areas[tag]+=area(q)
     for j in range(1,len(q)-1):
      tri=np.array([q[0],q[j],q[j+1]])
      if area(tri)<1e-9:continue
      w=np.array([inv@np.r_[v,1]for v in tri]);assert w.min()>-1e-6;batches.setdefault(tag,[]).append((ti,w))
   for tag,rows in batches.items():
    tri_ids=np.array([r[0]for r in rows]);weights=np.array([r[1]for r in rows]);q=copy.deepcopy(p);q['attributes']={};q.pop('extensions',None);source_indices=ids[tri_ids];attribute_errors={}
    for name,values in attrs.items():
     values=values[:,None]if values.ndim==1 else values;out=np.einsum('tij,tjk->tik',weights,values[source_indices]).reshape(-1,values.shape[1]);encoded=out.astype('<f4');error=float(np.max(abs(encoded-out)));attribute_errors[name]=error;assert error<(0.001 if name=='POSITION'else 2e-7),(name,error);q['attributes'][name]=accessor(encoded,'VEC'+str(values.shape[1]));
    q['indices']=accessor(np.arange(len(rows)*3),'SCALAR');groups.setdefault(tag,[]).append(q);key=f'node-{index}-primitive-{pi}-{tag}';all_provenance[key+'_triangles']=tri_ids;all_provenance[key+'_weights']=weights;proof.append(dict(component=tag,triangles=len(rows),attribute_errors=attribute_errors,provenance_prefix=key));total_dst+=len(rows)
   total_src+=len(ids);guards.append(dict(node=index,name=n['name'],primitive=pi,source_triangles=len(ids),split_triangles=splits,maximum_projected_area_error=maxerr,groups=proof))
  if 'outside'in groups:idx=len(doc['meshes']);doc['meshes'].append(dict(name=n['name']+' outside apertures',primitives=groups.pop('outside')));n['mesh']=idx
  else:n.pop('mesh',None)
  for tag,primitives in groups.items():
   h=next(h for h in holes if h['id']==tag);mi=len(doc['meshes']);doc['meshes'].append(dict(name=n['name']+' '+tag,primitives=primitives));ni=len(doc['nodes']);doc['nodes'].append(dict(name=n['name']+' / '+tag,mesh=mi,extras=dict(aperture_component=tag,aperture_source_node=index,reveal_hide_when_applied=h['triggers'])));n.setdefault('children',[]).append(ni)
  records.append(dict(source_node=index,source_name=n['name'],replacement_node=new,source_triangles=total_src,partition_triangles=total_dst));return new
 newroots=[clone(i,np.eye(4))for i in root_ids]
 def wrapper(children,label):
  group=len(doc['nodes']);doc['nodes'].append(dict(name=label+' placement',children=children,translation=offset.tolist()));i=len(doc['nodes']);doc['nodes'].append(dict(name=label,children=[group],rotation=[-math.sqrt(.5),0,0,math.sqrt(.5)]));return i
 doc['scenes']=[dict(name='current-original',nodes=[wrapper(root_ids,'Current original '+asset)]),dict(name='current-apertures',nodes=[wrapper(newroots,'Current apertures '+asset)])];doc['scene']=0;doc['buffers']=[dict(byteLength=len(binary))];binary.extend(b'\0'*(-len(binary)%4));payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*(-len(payload)%4);dest=DEST/(asset+'.glb');dest.write_bytes(struct.pack('<III',0x46546c67,2,28+len(payload)+len(binary))+struct.pack('<II',len(payload),0x4e4f534a)+payload+struct.pack('<II',len(binary),0x004e4942)+binary)
 decoded,check,_=la.read_glb(dest);assert hashlib.sha256(check[0][:len(buffers[0])]).hexdigest()==original_binary_sha;assert decoded['materials']==original['materials']and decoded.get('images')==original.get('images')and decoded.get('textures')==original.get('textures')and decoded.get('samplers')==original.get('samplers');assert sha(path)==source['model_sha256'];np.savez_compressed(DEST/(asset+'-provenance.npz'),**all_provenance)
 return dict(asset=asset,source=pin(path),descriptor=pin(LIB/source['descriptor']),placement=placement,world_zup_offset=offset.tolist(),model=pin(dest),original_scene='current-original',partition_scene='current-apertures',records=records,guards=guards,cap_projected_areas=cap_areas,all_original_buffer_bytes_preserved=True,materials_images_textures_samplers_exact=True,provenance=pin(DEST/(asset+'-provenance.npz')))

def main():
 DEST.mkdir(exist_ok=False);mp=LIB/'scenes/croisement02.rhlos-map.json';m=read(mp);packetp=BASE/'restart10-hole-aperture/packet-v1/packet.json';packet=read(packetp);holes=packet['holes'];assert len(holes)==17;plan=read(BASE/'restart38-approved-hole-export-v1/reader-config.json');assert {p['id']for p in plan['bindings']}=={t for h in holes for t in h['triggers']};results=[]
 for asset in ['croisement02-terrain','croisement02-north-woodland-bank']:
  source=next(s for s in m['sceneAssets']+m['assetSources']if s['id']==asset);assert sha(LIB/source['descriptor'])==source['descriptor_sha256'];place=dict(dx=0,dy=0,dz=0,rot_deg=0)if asset.endswith('terrain')else next(p for p in m['placements']if asset in p['assets'])['transform'];selected=[h for h in holes if (h['support_z']==0)==asset.endswith('terrain')];results.append(partition(asset,source,place,selected))
 for result in results:
  for id,value in result['cap_projected_areas'].items():assert abs(value-area(next(h['ellipse']for h in holes if h['id']==id)))<.05,(id,value)
 report=dict(status='CURRENT_RECEIVERS_PARTITIONED_PRIVATE_CPU_GUARDS_PASS',map=pin(mp),boundary_authority=pin(packetp),approved_endpoints_config=pin(BASE/'restart38-approved-hole-export-v1/reader-config.json'),holes=holes,receivers=results,geometry_scope='Coplanar surface subdivision only. Existing aperture screen boundaries and all source texture bytes retained.',canonical_writes=False,browser_verified=False,recipe=pin(Path(__file__).resolve()));(DEST/'receiver-partition.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status=report['status'],receivers=len(results),sites=len(holes))),flush=True)
if __name__=='__main__':main()
