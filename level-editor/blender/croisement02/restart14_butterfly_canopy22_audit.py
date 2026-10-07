"""Read-only native-ray context audit with transformed GLB geometry and alpha testing."""
import json,struct,hashlib,math,shutil,io,mmap
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial.transform import Rotation
from scipy.spatial import cKDTree
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/croisement02-refinement';B=W/'restart14-butterflies';O=B/'canopy22-alpha-audit-v2';LIB=ROOT/'level-editor/library'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def guard():
 used=sum(p.stat().st_size for p in O.rglob('*')if p.is_file())if O.exists()else 0
 assert used<8*1024**2 and shutil.disk_usage(ROOT).free>=8*1024**3+8*1024**2-used,'8GiB plus remaining8MiB extraction reserve'
def main(ray_records=None,postprocess=None,output=None):
 global O
 if output is not None:O=output
 guard();plan=json.loads((B/'all7-context-plan-v1/plan.json').read_text());mp=LIB/'scenes/croisement02.rhlos-map.json';assert sha(mp)==plan['map_sha256'];m=json.loads(mp.read_text());sources={r['id']:r for r in m['assetSources']};rays=[]
 for s in plan['sequences']:
  for f in s['path']:
   assert sha(Path(f['source']))==f['sha256'];rays.append({'sequence':s['index'],'phase':f['phase'],'screen':f['alpha_centroid_display'],'hits':[]})
 conf=json.loads((B/'flight-path-constraints-v1/proposal.json').read_text());keys={(r['sequence'],p['phase'])for r in conf['rows']for p in r['firsthit_canopy_conflicts']};rays=[r for r in rays if(r['sequence'],r['phase'])in keys];assert len(rays)==22
 if ray_records is not None:rays=ray_records
 q=np.array([r['screen'] for r in rays]);records=[];placements=[*m['placements'],*({'id':r['id'],'assets':[r['id']],'transform':{'dx':0,'dy':0,'dz':0,'rot_deg':0}} for r in m['sceneAssets'])];sources.update({r['id']:r for r in m['sceneAssets']})
 for placed in placements:
  guard();src=sources[placed['assets'][0]];path=LIB/src['model'];fd=path.open('rb');raw=mmap.mmap(fd.fileno(),0,access=mmap.ACCESS_READ);n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);offset=28+n;t=placed['transform'];assert t['rot_deg']==0;world=np.eye(4);world[:3,3]=[t['dx'],t['dz'],t['dy']/SIN];
  if src.get('role')=='ground':world[:3,:3]=Rotation.from_euler('x',-90,degrees=True).as_matrix()
  hidden={k for k,v in placed.get('parts',{}).items() if v.get('hidden')};textures={};external={};touched=0
  def acc(i):
   a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];assert 'sparse'not in a;dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']]);w={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];bi=v.get('buffer',0)
   if bi and bi not in external:external[bi]=(path.parent/doc['buffers'][bi]['uri']).read_bytes()
   return np.ndarray((a['count'],w),dtype=dt,buffer=external[bi]if bi else raw,offset=(0 if bi else offset)+v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*w),dt.itemsize)).copy()
  def alpha(mat,uv,linear=False):
   mode=mat.get('alphaMode','OPAQUE')
   if mode=='OPAQUE':return 1.
   p=mat.get('pbrMetallicRoughness',{});value=p.get('baseColorFactor',[1,1,1,1])[3];tex=p.get('baseColorTexture')
   if tex:
    assert not tex.get('extensions'),'Unsupported UV transform';idx=tex['index'];td=doc['textures'][idx];ii=td['source']
    if ii not in textures:
     image=doc['images'][ii];bv=doc['bufferViews'][image['bufferView']];data=raw[offset+bv.get('byteOffset',0):offset+bv.get('byteOffset',0)+bv['byteLength']];textures[ii]=np.array(Image.open(io.BytesIO(data)).convert('RGBA'))
    arr=textures[ii];sam=doc.get('samplers',[])[td['sampler']]if'sampler'in td else {};coords=[]
    for k,key in enumerate(['wrapS','wrapT']):
     c=float(uv[k]);wrap=sam.get(key,10497);c=np.clip(c,0,1-1e-10)if wrap==33071 else c%1 if wrap==10497 else 1-abs(c%2-1);coords.append(c)
    if linear:
     xy=np.array(coords)*[arr.shape[1],arr.shape[0]]-.5;ij=np.floor(xy).astype(int);frac=xy-ij;sample=0.
     for oy in [0,1]:
      for ox in [0,1]:
       at=[]
       for axis,k in enumerate([ox,oy]):
        size=arr.shape[1-axis];ix=ij[axis]+k;wrap=sam.get(['wrapS','wrapT'][axis],10497);ix=int(np.clip(ix,0,size-1))if wrap==33071 else ix%size if wrap==10497 else min(ix%(2*size),2*size-1-ix%(2*size));at.append(ix)
       sample+=arr[at[1],at[0],3]/255*(frac[0]if ox else 1-frac[0])*(frac[1]if oy else 1-frac[1])
     value*=sample
    else:value*=arr[min(int(coords[1]*arr.shape[0]),arr.shape[0]-1),min(int(coords[0]*arr.shape[1]),arr.shape[1]-1),3]/255
   return float(value)
  def visit(ni,parent,inherited_hidden=False):
   nonlocal touched
   node=doc['nodes'][ni];hide=inherited_hidden or node.get('name') in hidden;local=np.array(node['matrix']).reshape(4,4).T if'matrix'in node else np.eye(4)
   if'matrix'not in node:
    local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
   transform=parent@local
   if'mesh'in node and not hide and not node.get('extras',{}).get('gameplay_only'):
    for pi,prim in enumerate(doc['meshes'][node['mesh']]['primitives']):
     assert prim.get('mode',4)==4
     ai=prim['attributes']['POSITION'];a=doc['accessors'][ai];lo=a['min'];hi=a['max'];corners=np.array([[x,y,z,1]for x in [lo[0],hi[0]]for y in [lo[1],hi[1]]for z in [lo[2],hi[2]]])@transform.T;screen=np.c_[corners[:,0],SIN*corners[:,2]-COS*corners[:,1]];eligible=np.flatnonzero(np.all(q>=screen.min(0),axis=1)&np.all(q<=screen.max(0),axis=1))
     if not len(eligible):continue
     v=acc(ai);v=(np.c_[v,np.ones(len(v))]@transform.T)[:,:3];ind=acc(prim['indices']).ravel()if'indices'in prim else np.arange(len(v));tri=v[ind].reshape(-1,3,3);st=np.stack((tri[:,:,0],SIN*tri[:,:,2]-COS*tri[:,:,1]),axis=2);uvs=acc(prim['attributes']['TEXCOORD_0'])[ind].reshape(-1,3,2)if'TEXCOORD_0'in prim['attributes']else None;mat=doc.get('materials',[])[prim['material']]if'material'in prim else {};touched+=1
     tc=mat.get('pbrMetallicRoughness',{}).get('baseColorTexture',{}).get('texCoord',0);uvkey=f'TEXCOORD_{tc}'
     if uvkey in prim['attributes']:uvs=acc(prim['attributes'][uvkey])[ind].reshape(-1,3,2)
     colors=None
     if 'COLOR_0' in prim['attributes']:
      ci=prim['attributes']['COLOR_0'];colors=acc(ci).astype(float);ca=doc['accessors'][ci]
      if ca.get('normalized'):colors/=float({5121:255,5123:65535}[ca['componentType']])
      colors=colors[ind].reshape(-1,3,colors.shape[1])
     normals=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]);facing=normals@np.array([0,SIN,COS])*np.sign(np.linalg.det(transform[:3,:3]))
     aa=st[:,0];bb=st[:,1]-aa;cc=st[:,2]-aa;det=bb[:,0]*cc[:,1]-bb[:,1]*cc[:,0];valid=abs(det)>1e-10
     centroids=st.mean(axis=1);radius=np.linalg.norm(st-centroids[:,None,:],axis=2).max();tree=cKDTree(centroids);near=tree.query_ball_point(q[eligible],radius+1e-7)
     for ri,candidates in zip(eligible,near):
      ids=np.array(candidates,dtype=int)
      if not len(ids):continue
      dd=q[ri]-aa[ids];dv=det[ids];ok=valid[ids];u=np.divide(dd[:,0]*cc[ids,1]-dd[:,1]*cc[ids,0],dv,out=np.zeros(len(ids)),where=ok);vv=np.divide(bb[ids,0]*dd[:,1]-bb[ids,1]*dd[:,0],dv,out=np.zeros(len(ids)),where=ok);ii=np.flatnonzero(ok&(u>=0)&(vv>=0)&(u+vv<=1))
      for local_index in ii:
       ti=ids[local_index]
       weights=np.array([1-u[local_index]-vv[local_index],u[local_index],vv[local_index]]);uv=weights@uvs[ti]if uvs is not None else np.zeros(2);vertex=weights@colors[ti]if colors is not None else np.ones(4);vertex_alpha=float(vertex[3])if len(vertex)==4 else 1.;al=alpha(mat,uv)*(vertex_alpha if mat.get('alphaMode','OPAQUE')!='OPAQUE'else 1);cut=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
       linear_alpha=alpha(mat,uv,True);passing=linear_alpha>=cut and (facing[ti]>0 or mat.get('doubleSided',False))
       pos=weights@tri[ti];rays[ri]['hits'].append({'asset':placed['id'],'node':node.get('name'),'world_yup':pos.tolist(),'camera_depth':float(COS*pos[2]+SIN*pos[1]),'alpha':al,'bilinear_level0_runtime_alpha':linear_alpha,'passes_alpha_and_culling':bool(passing),'passes_alpha_only':bool(linear_alpha>=cut),'vertex_rgba':vertex.tolist(),'uv':uv.tolist(),'barycentric':weights.tolist(),'node_index':ni,'mesh_index':node['mesh'],'primitive_index':pi,'triangle_index':int(ti),'material_index':prim.get('material'),'material_name':mat.get('name'),'texture_index':mat.get('pbrMetallicRoughness',{}).get('baseColorTexture',{}).get('index'),'front_facing_dot':float(facing[ti]),'double_sided':mat.get('doubleSided',False)})
   for child in node.get('children',[]):visit(child,transform,hide)
  for ni in doc['scenes'][doc.get('scene',0)]['nodes']:visit(ni,world)
  if touched:records.append({'asset':placed['id'],'model_sha256':sha(path),'primitives_tested':touched})
  raw.close();fd.close();print('ASSET',placed['id'],touched,flush=True)
 for r in rays:
  r['hits'].sort(key=lambda v:v['camera_depth'],reverse=True);passing=[v for v in r['hits']if v['passes_alpha_and_culling']];r['first_hit']=passing[0]if passing else None;r['height_hypotheses']=[{'z':z,'native_front_of_first_hit':r['first_hit']is None or (COS*r['screen'][1]/SIN+z/SIN)>r['first_hit']['camera_depth']}for z in [10,30,60,100,150]]
 if postprocess is not None:return postprocess(rays,records,mp)
 O.mkdir(exist_ok=True);(O/'report.json').write_text(json.dumps({'status':'READ_ONLY_CENTER_RAYS_NOT_COLLISION_COMPLETION','map_sha256':sha(mp),'source_count':693,'selected_center_rays':22,'disk_override':'Root authorizes readonlyCPU8GiB+remaining8MiB underrestart17policy; no model/render lane','policy_sha256':sha(W/'restart17-small-job-disk-policy.json'),'records':records,'rays':rays,'limits':['Nearest vertex-inclusive alpha and runtime bilinearlevel0 alpha both recorded. Runtime foliage ignores vertex alpha; mip filtering remains view-dependent.','Centroid rays only; body registration and entire wing swept volume not yet tested.','Height proposals are inferred, not source elevation.','Static installed active placements only; native FX layering remains separate.']},indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<8*1024**2
if __name__=='__main__':main()
