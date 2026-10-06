"""Small, verified initial-rig delivery with flattened dual-UV appearance."""
import sys,json,struct,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import pixels
from refinement_review import _tree
from tree_geometry import RAY,SIN
from render_multiview_asset import render
ROOT=OUT/'restart5-initial-nets'
DEST=ROOT/'approved-delivery-v1'
HASHES={'00':'7541523af2a34f6db77820a375d1c3202e8752e9b5cd9cf74eb15ee8528b96d8','01':'1983c0ea22240086e82840a00632de247e82da84564b6288715311447733a204'}
APPROVAL=OUT/'restart3-review-batches/batch-v10/user-approval.json'
def guard():
 assert shutil.disk_usage(OUT).free>23*1024**3,'Free-space guard'
 total=sum(p.stat().st_size for p in DEST.rglob('*')if p.is_file())if DEST.exists()else 0
 assert total<50*1024**2,('New aggregate output cap',total)
 return total
def linear(a):return np.where(a<=.04045,a/12.92,((a+.055)/1.055)**2.4)
def encoded(a):return np.where(a<=.0031308,a*12.92,1.055*np.maximum(a,0)**(1/2.4)-.055)
def interp(a,xy):
 h,w=a.shape[:2];coords=np.array([xy[:,1]*h-.5,xy[:,0]*w-.5])
 return np.stack([map_coordinates(a[:,:,c],coords,order=1,mode='nearest')for c in range(a.shape[2])],axis=1)
def flatten(obj):
 generated=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'))
 nd=next(n for n in generated.node_tree.nodes if n.type=='TEX_IMAGE'and n.image)
 uvname=nd.inputs['Vector'].links[0].from_node.uv_map
 original=pixels(nd.image);h,w=original.shape[:2];scale=4;hh,ww=h*scale,w*scale
 # Work row-wise to keep the broad ground atlas's temporary memory bounded.
 result=np.empty((hh,ww,4),np.float32)
 for y in range(hh):
  q=np.column_stack(((np.arange(ww)+.5)/ww,np.full(ww,(y+.5)/hh)))
  result[y]=interp(original,q)
 generated_uv=obj.data.uv_layers[uvname];native_uv=obj.data.uv_layers['Initial native source projection']
 native_node=next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE'and n.image)
 native=pixels(native_node.image);nh,nw=native.shape[:2];obj.data.calc_loop_triangles();faces=0
 for t in obj.data.loop_triangles:
  mat=obj.data.materials[t.material_index]
  if not any(n.type=='MIX_RGB'for n in mat.node_tree.nodes):continue
  faces+=1;q=np.array([generated_uv.data[i].uv[:]for i in t.loops]);nq=np.array([native_uv.data[i].uv[:]for i in t.loops]);p=q*[ww,hh]
  m=np.column_stack((p[1]-p[0],p[2]-p[0]));det=np.linalg.det(m)
  if abs(det)<1e-10:continue
  lo=np.maximum(0,np.floor(p.min(0)).astype(int));hi=np.minimum([ww-1,hh-1],np.floor(p.max(0)).astype(int))
  if (hi<lo).any():continue
  yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];xy=np.column_stack((xx.ravel(),yy.ravel()));b=(xy+.5-p[0])@np.linalg.inv(m).T
  inside=(b[:,0]>=-1e-7)&(b[:,1]>=-1e-7)&(b.sum(1)<=1+1e-7);xy=xy[inside];b=b[inside]
  if not len(xy):continue
  nuv=nq[0]+b[:,0,None]*(nq[1]-nq[0])+b[:,1,None]*(nq[2]-nq[0]);valid=(nuv>=0).all(1)&(nuv<1).all(1)
  xy=xy[valid];nuv=nuv[valid];nx=np.floor(nuv[:,0]*nw).astype(int);ny=np.floor(nuv[:,1]*nh).astype(int);src=native[ny,nx];old=result[xy[:,1],xy[:,0]]
  rgb=encoded(linear(src[:,:3])*src[:,3,None]+linear(old[:,:3])*(1-src[:,3,None]));result[xy[:,1],xy[:,0],:3]=rgb
 result[:,:,3]=1
 return dict(array=np.clip(np.rint(result*255),0,255).astype(np.uint8),uv=uvname,faces=faces,native=native)
def triangles(objects):
 rows=[]
 for o in objects:
  o.data.calc_loop_triangles();rows.extend((o,t)for t in o.data.loop_triangles)
 return rows
def native_samples(objects,report,observed):
 tree,owners,_=_tree(objects);tris=triangles(objects);ox,oy=report['source']['origin'];rows=[]
 for y,x in np.argwhere(observed[:,:,3]>0):
  p,n,i,d=tree.ray_cast(Vector((ox+x+.5,-(oy+y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None
  o,t=tris[i];assert owners[i]==o
  rows.append((int(x),int(y),p,o,t))
 return rows
def main(key):
 guard();assert sha(APPROVAL)=='534d552590823cbb5221e1ff52a082657360eb16f80ff5c42acfdb4980a2e3e8'
 exp=ROOT/f'texture-fill-v1/profile-{key}/experiment';parent=exp/'unseen-complete-v2';assert sha(parent/'worker.blend')==HASHES[key]
 out=DEST/f'profile-{key}';out.mkdir(parents=True,exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent/'worker.blend'));scene=bpy.context.scene;meta=json.loads((exp/'views.json').read_text());asset=meta['asset_id']
 assert any(m['asset_id']==asset and m['model_sha256']==HASHES[key]for d in json.loads(APPROVAL.read_text())['decisions']for m in d['members'])
 objects=sorted([o for o in scene.objects if o.type=='MESH'],key=lambda o:o.name);parts=[];flat={}
 for o in objects:
  o.data.calc_loop_triangles();points=np.array([o.matrix_world@v.co for v in o.data.vertices]);parts.append(dict(name=o.name,vertices=len(points),triangles=len(o.data.loop_triangles),bounds=[points.min(0).tolist(),points.max(0).tolist()]));flat[o.name]=flatten(o)
 report=json.loads((parent/'report.json').read_text());observed=np.array(Image.open(ROOT/f'candidate-v5/profile-{key}/observed-source.png').convert('RGBA'));source=np.array(Image.open(report['source']['source']).convert('RGBA'));samples=native_samples(objects,report,observed);pins={};corrections=0
 for x,y,p,o,t in samples:
  f=flat[o.name];uv=o.data.uv_layers[f['uv']];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=f['array'];h,w=a.shape[:2];ix=min(w-1,max(0,int(q.x*w)));iy=min(h-1,max(0,int(q.y*h)));pin=(o.name,ix,iy);color=source[y,x]
  assert pin not in pins or np.array_equal(pins[pin],color),'Conflicting native centers in export atlas'
  pins[pin]=color
  if not np.array_equal(a[iy,ix],color):corrections+=1;a[iy,ix]=color
 for o in objects:
  f=flat[o.name];p=out/(o.name.replace(' ','-')+'.png');Image.fromarray(f['array'][::-1]).save(p);image=bpy.data.images.load(str(p),check_existing=False);mat=bpy.data.materials.new(o.name+' / export flattened native and generated');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();uv=nodes.new('ShaderNodeUVMap');uv.uv_map=f['uv'];tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='EXTEND';emit=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(uv.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Color'],emit.inputs['Color']);mat.node_tree.links.new(emit.outputs[0],output.inputs['Surface']);o.data.materials.clear();o.data.materials.append(mat)
  for face in o.data.polygons:face.material_index=0
  o.data.uv_layers.active=o.data.uv_layers[f['uv']]
  # Dense export UV channels avoid GLTF primitive attribute limits.
  for layer in list(o.data.uv_layers):
   if layer.name!=f['uv']:o.data.uv_layers.remove(layer)
  matrix=o.matrix_world.copy();o.parent=None;o.matrix_world=matrix
 bpy.ops.object.select_all(action='DESELECT')
 for o in objects:o.hide_set(False);o.select_set(True)
 target=out/'model.glb';bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',use_selection=True,export_animations=False,export_yup=True,export_materials='EXPORT',export_extras=True)
 b=target.read_bytes();n,kind=struct.unpack_from('<II',b,12);doc=json.loads(b[20:20+n]);tail=b[20+n:];family='net-piege01'if key=='00'else'net-piege03';origins=OUT/'restart2-state/remaining-local-origins-v1/manifest.json';anchor=json.loads(origins.read_text())['anchors'][family]
 assert all('KHR_materials_unlit'in m.get('extensions',{})for m in doc['materials']);assert all('baseColorTexture'in m['pbrMetallicRoughness']for m in doc['materials'])
 sc=doc['scenes'][doc.get('scene',0)];node=len(doc['nodes']);doc['nodes'].append(dict(name='Reusable family origin',translation=[-v for v in anchor],children=sc['nodes']));sc['nodes']=[node];j=json.dumps(doc,separators=(',',':')).encode();j+=b' '*((-len(j))%4);data=struct.pack('<III',0x46546c67,2,20+len(j)+len(tail))+struct.pack('<II',len(j),kind)+j+tail;target.write_bytes(data);assert data[20+len(j):]==tail;guard()
 # Reopen the actual GLB in the same review scene, restoring only its binding.
 for o in objects:bpy.data.objects.remove(o,do_unlink=True)
 before=set(scene.objects);bpy.ops.import_scene.gltf(filepath=str(target));new=[o for o in scene.objects if o not in before];root=next(o for o in new if o.name.startswith('Reusable family origin'));root.location+=Vector((anchor[0],-anchor[2],anchor[1]));bpy.context.view_layer.update();imported=sorted([o for o in new if o.type=='MESH'],key=lambda o:o.name)
 assert len(imported)==3;import_parts=[]
 for o,part in zip(imported,parts):
  assert o.name==part['name'];o.data.calc_loop_triangles();assert len(o.data.loop_triangles)==part['triangles'];points=np.array([o.matrix_world@v.co for v in o.data.vertices]);bounds=np.array([points.min(0),points.max(0)]);assert np.max(abs(bounds-np.array(part['bounds'])))<.001;o['asset_group']=asset;import_parts.append(dict(name=o.name,bounds=bounds.tolist(),triangles=len(o.data.loop_triangles)))
 exact=0
 for x,y,p,o,t in native_samples(imported,report,observed):
  mat=o.data.materials[t.material_index];nd=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);uv=o.data.uv_layers[0];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=np.rint(pixels(nd.image)*255).astype(np.uint8);h,w=a.shape[:2];color=a[min(h-1,max(0,int(q.y*h))),min(w-1,max(0,int(q.x*w)))];assert np.array_equal(color,source[y,x]),(x,y,color.tolist(),source[y,x].tolist());exact+=1
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(exp/'views.json',out/'actual',width=384);sheet=Image.new('RGBA',(1536,768))
 for i in range(8):sheet.paste(Image.open(out/'actual'/f'view-{i}-textured.png').convert('RGBA'),((i%4)*384,(i//4)*384))
 sheet.save(out/'actual/textured.png');assert sha(parent/'worker.blend')==HASHES[key]
 write_json(out/'export.json',dict(status='PASS technical; independent visual review pending',asset_id=asset,profile='Trapcr02-'+key,model_source=str(parent/'worker.blend'),model_sha256=HASHES[key],approval_sha256=sha(APPROVAL),glb=str(target),glb_sha256=sha(target),family=family,position=anchor,family_origins_sha256=sha(origins),binary_chunks_unchanged_by_rebase=True,binary_chunks_sha256=hashlib.sha256(tail).hexdigest(),parts=parts,imported_parts=import_parts,native_exact_RGBA=exact,native_atlas_center_corrections=corrections,atlas_scale=4,limits=['Export shader flattened at four times generated atlas dimensions; nearest resampling approximates source boundaries away from exact native center witnesses.','Shape/source approved models unchanged. No mission state or gameplay edits.','Native-only uncertain source fragments remain excluded.'],new_aggregate_bytes=guard()))
 print('EXPORTED',key,sha(target),exact,flush=True)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
