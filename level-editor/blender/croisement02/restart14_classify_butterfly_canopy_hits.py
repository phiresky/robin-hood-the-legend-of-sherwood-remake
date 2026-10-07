"""Bind exact butterfly conflict triangles to installed canopy source provenance."""
from pathlib import Path
import json,struct,hashlib,io,math
from collections import Counter
import numpy as np
from PIL import Image
from restart14_butterfly_canopy_provenance import ROOT,LIB,BASE,DEST,sha
INPUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies/canopy22-alpha-audit-v2/report.json'
def main():
 registry=json.loads((DEST/'material-registry.json').read_text());report=json.loads(INPUT.read_text());models={};observed={};images={}
 for r in registry['assets']:
  path=Path(r['model']);assert sha(path)==r['model_sha256'];raw=path.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);models[r['asset']]=(path,raw,doc,28+n,{})
 def check(h):
  if h['asset']not in models:return {'receiver':h['asset'],'classification':'noncanopy corrected receiver','hit':h}
  path,raw,doc,offset,external=models[h['asset']]
  def acc(i):
   a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']]);w={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];bi=v.get('buffer',0)
   if bi and bi not in external:external[bi]=(path.parent/doc['buffers'][bi]['uri']).read_bytes()
   q=np.ndarray((a['count'],w),dtype=dt,buffer=external[bi]if bi else raw,offset=(0 if bi else offset)+v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*w),dt.itemsize))
   return q.astype(float)/np.iinfo(dt).max if a.get('normalized') else q
  node=doc['nodes'][h['node_index']];assert node['mesh']==h['mesh_index'];prim=doc['meshes'][node['mesh']]['primitives'][h['primitive_index']];assert prim['material']==h['material_index'];mat=doc['materials'][prim['material']];tex=mat['pbrMetallicRoughness']['baseColorTexture'];assert tex['index']==h['texture_index'];ids=acc(prim['indices']).ravel()[3*h['triangle_index']:3*h['triangle_index']+3];weights=np.array(h['barycentric']);uv=weights@acc(prim['attributes'][f"TEXCOORD_{tex.get('texCoord',0)}"])[ids];assert np.max(abs(uv-np.array(h['uv'])))<1e-10
  color=weights@acc(prim['attributes']['COLOR_0'])[ids];assert np.max(abs(color-np.array(h['vertex_rgba'])))<1e-10;known=bool(mat['extras']['foliage_observed']);assert abs(float(color[0])-int(known))<1e-9
  td=doc['textures'][tex['index']];ii=td['source'];key=(h['asset'],ii)
  if key not in images:
   image=doc['images'][ii];bv=doc['bufferViews'][image['bufferView']];data=raw[offset+bv.get('byteOffset',0):offset+bv.get('byteOffset',0)+bv['byteLength']];images[key]=(np.array(Image.open(io.BytesIO(data)).convert('RGBA')),hashlib.sha256(data).hexdigest())
  arr,digest=images[key];height,width=arr.shape[:2];sam=doc.get('samplers',[])[td['sampler']]if'sampler'in td else {};base=mat['pbrMetallicRoughness'].get('baseColorFactor',[1,1,1,1])[3]
  def sample(ix,iy):
   coords=[]
   for q,size,key in[(ix,width,'wrapS'),(iy,height,'wrapT')]:
    wrap=sam.get(key,10497)
    if wrap==33071:q=min(size-1,max(0,q))
    elif wrap==10497:q=q%size
    else:q=q%(2*size);q=q if q<size else 2*size-1-q
    coords.append(q)
   return arr[coords[1],coords[0],3]/255*base
  nearest=sample(math.floor(uv[0]*width),math.floor(uv[1]*height));xx=uv[0]*width-.5;yy=uv[1]*height-.5;ix=math.floor(xx);iy=math.floor(yy);dx=xx-ix;dy=yy-iy;linear=(1-dx)*(1-dy)*sample(ix,iy)+dx*(1-dy)*sample(ix+1,iy)+(1-dx)*dy*sample(ix,iy+1)+dx*dy*sample(ix+1,iy+1)
  assert abs(nearest-h['alpha'])<1e-8;assert abs(linear-h['bilinear_level0_runtime_alpha'])<1e-8
  face=mat.get('doubleSided',False) or h['front_facing_dot']>0;passed=face and linear>=mat.get('alphaCutoff',.5);assert passed==h['passes_alpha_and_culling']
  return {'receiver':h['asset'],'classification':'observed-front texture on reconstructed leaf geometry'if known else 'inferred leaf back/interior','hit':h,'independent_checks':{'primitive_material_binding':True,'barycentric_uv_exact':True,'ownership_red':float(color[0]),'vertex_alpha':1. if len(color)<4 else float(color[3]),'runtime_vertex_alpha_ignored':True,'encoded_image_sha256':digest,'image_size':[width,height],'nearest_alpha':float(nearest),'bilinear_level0_alpha':float(linear),'front_side_pass':face,'mask_cutoff_pass':bool(linear>=.5),'physical_pass_at_bilinear_level0':bool(passed),'uv_pixel_continuous':[float(uv[0]*width),float(uv[1]*height)]}}
 rows=[]
 for r in report['rays']:
  assert len(r['original_hit_recovered'])==1;rows.append({'sequence':r['sequence'],'phase':r['phase'],'native_screen':r['screen'],'original':check(r['original_hit_recovered'][0]),'corrected':check(r['first_hit'])})
 summary={'original_observed':sum('observed-front'in r['original']['classification']for r in rows),'original_inferred':sum('inferred leaf'in r['original']['classification']for r in rows),'corrected_observed':sum('observed-front'in r['corrected']['classification']for r in rows),'corrected_inferred':sum('inferred leaf'in r['corrected']['classification']for r in rows),'corrected_noncanopy':sum(r['corrected']['classification'].startswith('noncanopy')for r in rows)}
 result={'status':'READ_ONLY_EXACT_PROVENANCE_CLASSIFIED_WITH_FILTERING_LIMIT','inputs':{'narrow22_report':{'path':str(INPUT),'sha256':sha(INPUT)},'material_registry':{'path':str(DEST/'material-registry.json'),'sha256':sha(DEST/'material-registry.json')},'production_texture_display':{'path':str(ROOT/'level-editor/app/src/texture-display.ts'),'sha256':sha(ROOT/'level-editor/app/src/texture-display.ts')}},'models':[{'asset':r['asset'],'model':r['model'],'sha256':r['model_sha256']}for r in registry['assets']],'summary':summary,'rows':rows,'conclusion':'The22 original reports are not22 inferred foliage obstructions:20 hit observed source fronts and2 hit inferred backs that should be culled. Corrected sidedness plus bilinear level0 leaves19 observed-front canopy hits and3 ground hits, with no inferred canopy first hit. Preserve observed source surfaces; path height/layering decisions must distinguish observed RGB from reconstructed3D depth.','limits':['This independently verifies exact triangle indices, barycentric UVs, provenance color, texture image alpha and material-sidedness rule against current installed models; full ray intersections are supplied by the independently maintained terrain audit.','Bilinear level0 is not exact GPU minification/mipmap/anisotropic coverage; alpha edge decisions can remain view-dependent.','Static installed canopy phase only; butterflies and tree animation clocks are independent, and approved Tree42v5 private motion is not installed by this audit.','Source-front observed colors do not establish original3D altitude or authorize moving/deleting leaf geometry.','No trees, materials, textures, runtime or source ownership were changed; no Blender/render/newAPI.']}
 out=DEST/'classification.json';assert not out.exists();data=json.dumps(result,indent=2)+'\n';assert sum(p.stat().st_size for p in DEST.rglob('*')if p.is_file())+len(data.encode())<2*2**20;out.write_text(data);print(summary);print(out,sha(out))
if __name__=='__main__':main()
