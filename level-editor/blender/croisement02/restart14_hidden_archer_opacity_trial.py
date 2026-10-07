"""Private conservative alpha-only test on proven all-frame canopy gaps."""
import sys,json,io,hashlib,math
from pathlib import Path
import numpy as np
from PIL import Image
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS

def signature(obj):
 h=hashlib.sha256()
 for v in obj.data.vertices:h.update(np.asarray(v.co,dtype='<f4').tobytes())
 for p in obj.data.polygons:h.update(np.asarray(p.vertices,dtype='<i4').tobytes())
 for uv in obj.data.uv_layers:
  h.update(uv.name.encode());a=np.zeros(len(uv.data)*2,dtype=np.float32);uv.data.foreach_get('uv',a);h.update(a.tobytes())
 h.update(np.asarray(obj.matrix_world,dtype='<f4').tobytes());return h.hexdigest()
def main():
 audit=OUT/'restart14-hidden-archer/audit-v1';dest=OUT/'restart14-hidden-archer/crown-opacity-trial-v1';dest.mkdir(exist_ok=False)
 proof=audit/'source-role-join-v1/animation-alpha-history.json';history=json.loads(proof.read_text());pixels={tuple(s['pixel']) for r in history['records'] for s in r['samples'] if max(s['frame_alpha'])==0};allowed=np.zeros((1152,1792),bool)
 for x,y in pixels:allowed[y,x]=True
 source=json.loads((audit/'crown-provenance-v2.json').read_text());assert sha(source['source'])==source['source_sha256'];bpy.ops.wm.open_mainfile(filepath=source['source']);scene=bpy.data.scenes.new('Hidden archer private opacity experiment');bpy.context.window.scene=scene
 names={s['object'] for p in source['profiles'] for s in p['samples']};records=[]
 parents={str(Path(bpy.path.abspath(l.filepath)).resolve()):sha(Path(bpy.path.abspath(l.filepath)).resolve()) for l in bpy.data.libraries}
 for name in sorted(names):
  parent=bpy.data.objects[name];obj=parent.copy();obj.data=parent.data.copy();obj.parent=None;obj.matrix_world=parent.matrix_world.copy();scene.collection.objects.link(obj);obj.name=name+' / private all-frame gap experiment';before=signature(parent);assert signature(obj)==before;obj.data.calc_loop_triangles();materials=[]
  for mi,original in enumerate(list(obj.data.materials)):
   if not original or not original.get('foliage_physical_opacity'):continue
   mat=original.copy();obj.data.materials[mi]=mat;shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED');texture=shader.inputs['Alpha'].links[0].from_node;assert texture.type=='TEX_IMAGE';im=texture.image;assert im.packed_file;rgba=np.array(Image.open(io.BytesIO(im.packed_file.data)).convert('RGBA'));h,w=rgba.shape[:2];eligible=np.zeros((h,w),bool);blocked=np.zeros((h,w),bool);uvname=texture.inputs['Vector'].links[0].from_node.uv_map;uvs=obj.data.uv_layers[uvname]
   # Track every use of each texel. Shared donor texels remain protected when
   # even one receiving triangle lies outside the proven gap domain.
   for tri in obj.data.loop_triangles:
    if tri.material_index!=mi:continue
    uv=np.array([uvs.data[l].uv for l in tri.loops]);uv[:,0]*=w;uv[:,1]=(1-uv[:,1])*h;lo=np.maximum(0,np.floor(uv.min(0)).astype(int));hi=np.minimum([w,h],np.ceil(uv.max(0)).astype(int));
    if np.any(hi<=lo):continue
    aa,bb,cc=uv;matrix=np.column_stack([bb-aa,cc-aa]);det=np.linalg.det(matrix)
    if abs(det)<1e-10:continue
    yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];sample=np.stack([xx+.5,yy+.5],axis=-1);weights=(sample-aa)@np.linalg.inv(matrix).T;inside=(weights[:,:,0]>=-1e-5)&(weights[:,:,1]>=-1e-5)&(weights.sum(2)<=1.00001)&(rgba[lo[1]:hi[1],lo[0]:hi[0],3]>=128)
    if not inside.any():continue
    world=np.array([obj.matrix_world@obj.data.vertices[v].co for v in tri.vertices]);p=world[0]+weights[:,:,0,None]*(world[1]-world[0])+weights[:,:,1,None]*(world[2]-world[0]);sx=np.floor(p[:,:,0]).astype(int);sy=np.floor(-p[:,:,1]*SIN-p[:,:,2]*COS).astype(int);valid=(sx>=0)&(sx<1792)&(sy>=0)&(sy<1152);safe=np.zeros_like(valid);safe[valid]=allowed[sy[valid],sx[valid]];eligible[yy[inside&safe],xx[inside&safe]]=True;blocked[yy[inside&~safe],xx[inside&~safe]]=True
   change=eligible&~blocked;result=rgba.copy();result[change,3]=0;assert np.array_equal(result[:,:,:3],rgba[:,:,:3]);assert np.array_equal(result[~change],rgba[~change]);path=dest/f'{name.split(" / ")[0].replace(" ","-")}-material-{mi}.png';Image.fromarray(result).save(path);new=bpy.data.images.load(str(path),check_existing=False);new.pack();texture.image=new
   materials.append(dict(material_index=mi,parent_material=original.name,parent_image=im.filepath,parent_packed_sha256=hashlib.sha256(im.packed_file.data).hexdigest(),alpha=str(path),alpha_sha256=sha(path),changed_texels=int(change.sum()),shared_texels_protected=int((eligible&blocked).sum()),rgb_exact=True,outside_alpha_exact=True))
  assert signature(obj)==before;records.append(dict(parent_object=name,parent_geometry_uv_world_sha256=before,derivative_object=obj.name,derivative_geometry_uv_world_sha256=signature(obj),materials=materials))
 bpy.context.preferences.filepaths.save_version=0
 # Save just the private scene and its four copied receivers, not the full map.
 model=dest/'crowns.blend';bpy.data.libraries.write(str(model),{scene},fake_user=True,compress=True)
 write_json(dest/'preservation.json',dict(status='Private investigation only; no approved crown replaced',model=str(model),model_sha256=sha(model),source_scene=source['source'],source_scene_sha256=source['source_sha256'],source_libraries=parents,domain_proof=str(proof),domain_proof_sha256=sha(proof),unique_native_gap_coordinates=len(pixels),records=records,limits=['Only texel-center projection tested; boundary sampling and all-view effects require review.','Shared donor texels with any out-of-domain receiver remain unchanged.','Frame-dependent native alpha is excluded from this experiment.']))
 print([(r['parent_object'],sum(m['changed_texels'] for m in r['materials'])) for r in records],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
