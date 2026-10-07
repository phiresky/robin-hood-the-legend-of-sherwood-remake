"""Compare retained crown surfaces and material semantics with frozen installed assets."""
from pathlib import Path
import copy,hashlib,json,sys
import numpy as np
from scipy.spatial import cKDTree
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement/blender'))
from lossy_assets import read_glb,accessor_array
O=R/'level-editor/work/croisement02-refinement';B=O/'restart8-five-bark-approved-export-v1';OLD=O/'restart2-textures/post-batch15-static-candidate-v6/map-assets/3d-assets/croisement02'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def image_hash(path,d,b,i):
 im=d['images'][i]
 if 'uri'in im:data=(path.parent/im['uri']).read_bytes()
 else:
  v=d['bufferViews'][im['bufferView']];data=b[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]
 return hashlib.sha256(data).hexdigest()
def material(path,d,b,index):
 m=copy.deepcopy(d['materials'][index]);m.pop('name',None)
 def visit(v):
  if isinstance(v,dict):
   for k,x in list(v.items()):
    if k.endswith('Texture')and isinstance(x,dict)and'index'in x:
     tex=d['textures'][x.pop('index')];x['image_sha256']=image_hash(path,d,b,tex['source']);x['sampler']=d.get('samplers',[])[tex['sampler']]if'sampler'in tex else{}
    else:visit(x)
  elif isinstance(v,list):
   for x in v:visit(x)
 visit(m);return m
def without_samplers(v):
 if isinstance(v,dict):return{k:without_samplers(x)for k,x in v.items()if k!='sampler'}
 if isinstance(v,list):return[without_samplers(x)for x in v]
 return v
def main():
 rows=[]
 for n in [18,24,38,39,45]:
  a=OLD/f'croisement02-tree-{n}/model.glb';z=B/f'tree-{n}-v1/delivery-v3/model.glb';da,ba,_=read_glb(a);dz,bz,_=read_glb(z);na=next(x for x in da['nodes']if'Crown'in x.get('name',''));nz=next(x for x in dz['nodes']if'Crown'in x.get('name',''));pa=da['meshes'][na['mesh']]['primitives'];pz=dz['meshes'][nz['mesh']]['primitives'];assert len(pa)==len(pz);records=[]
  for k,(p,q)in enumerate(zip(pa,pz)):
   ma=material(a,da,ba,p['material']);mz=material(z,dz,bz,q['material']);assert without_samplers(ma)==without_samplers(mz),(n,k,'non-sampler material difference')
   xa=accessor_array(da,ba,p['attributes']['POSITION']);xz=accessor_array(dz,bz,q['attributes']['POSITION'])
   va=np.column_stack([xa,accessor_array(da,ba,p['attributes']['TEXCOORD_0'])*100,accessor_array(da,ba,p['attributes']['COLOR_0'])*10]);vz=np.column_stack([xz,accessor_array(dz,bz,q['attributes']['TEXCOORD_0'])*100,accessor_array(dz,bz,q['attributes']['COLOR_0'])*10]);dist,idx=cKDTree(va).query(vz,distance_upper_bound=.002);reverse=cKDTree(vz).query(va,distance_upper_bound=.002)[0];assert np.isfinite(dist).all()and np.isfinite(reverse).all(),(n,k,'surface/UV/color drift');ia=accessor_array(da,ba,p['indices']).reshape(-1,3);iz=accessor_array(dz,bz,q['indices']).reshape(-1,3);assert len(ia)==len(iz);face_dist=cKDTree(va[ia].mean(axis=1)).query(vz[iz].mean(axis=1),distance_upper_bound=.002)[0];assert np.isfinite(face_dist).all(),(n,k,'triangle surface drift')
   records.append(dict(primitive=k,maximum_surface_uv_color_distance=float(max(dist.max(),reverse.max())),maximum_triangle_center_distance=float(face_dist.max()),triangle_count=len(accessor_array(da,ba,p['indices']))//3,all_non_sampler_material_semantics_exact=True,sampler_change=ma!=mz,old_material=ma,new_material=mz))
  rows.append(dict(tree=n,baseline=str(a),baseline_sha256=sha(a),candidate=str(z),candidate_sha256=sha(z),primitives=records));print(n,sum(x['sampler_change']for x in records),flush=True)
 report=dict(status='HOLD crown sampler differences require private correction',scope='Frozen installed772 baseline vs independently reviewed five bark derivatives',trees=rows);(B/'live-crown-audit-v1.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
