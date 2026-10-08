"""Verify approved static-pair export native samples and bidirectional surface parity."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';TREE=int(sys.argv[sys.argv.index('--')+1]);assert TREE in (12,14);RAY=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));SIN=math.sin(math.radians(35))
def texture_ancestor(socket,seen=None):
 seen=set() if seen is None else seen
 for link in socket.links:
  node=link.from_node
  if node.type=='TEX_IMAGE':return node
  if node in seen:continue
  seen.add(node)
  for input in node.inputs:
   found=texture_ancestor(input,seen)
   if found:return found
 return None
def main():
 out=B/f'approved-hub-textures-v1/static-pair-export-v1/tree{TREE}';assert not (out/'native-proof.json').exists();source=B/f'tree{TREE}-static-leaf-crown-v3';acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source/'worker.blend'));original=[o for o in bpy.context.scene.objects if o.type=='MESH'];original_points=[];original_faces=[]
  for o in original:
   o.data.calc_loop_triangles();offset=len(original_points);original_points.extend([o.matrix_world@v.co for v in o.data.vertices]);original_faces.extend([[offset+i for i in t.vertices] for t in o.data.loop_triangles])
  original_bvh=BVHTree.FromPolygons(original_points,original_faces,all_triangles=True)
  bpy.ops.wm.read_factory_settings(use_empty=True);bpy.ops.import_scene.gltf(filepath=str(out/'model.glb'));report=json.loads((out/'report.json').read_text());shift=Matrix.Translation(Vector(report['export']['placement_origin_scene']));objects=list(bpy.context.scene.objects)
  for o in objects:
   if o.parent is None:o.matrix_world=shift@o.matrix_world
  bpy.context.view_layer.update();rows=[];import_points=[];import_faces=[]
  for o in objects:
   if o.type!='MESH':continue
   mesh=o.data;mesh.calc_loop_triangles();vs=[o.matrix_world@v.co for v in mesh.vertices];offset=len(import_points);import_points.extend(vs);import_faces.extend([[offset+i for i in t.vertices] for t in mesh.loop_triangles])
   for slot in {t.material_index for t in mesh.loop_triangles}:
    material=mesh.materials[slot];shader=next((n for n in material.node_tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
    color=texture_ancestor(shader.inputs['Emission Color']) if shader else texture_ancestor(next(n for n in material.node_tree.nodes if n.type=='EMISSION').inputs['Color']);
    if color is None and shader:color=texture_ancestor(shader.inputs['Base Color'])
    assert color
    alpha=texture_ancestor(shader.inputs['Alpha']) if shader else (color if material.get('foliage_physical_opacity') else None)
    def texdata(node):
     uv=mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map] if node.inputs['Vector'].links else mesh.uv_layers[0]
     rgba=np.asarray(node.image.pixels[:],np.float32).reshape(node.image.size[1],node.image.size[0],4)
     return uv,rgba,node.interpolation,node.extension
    tris=[t for t in mesh.loop_triangles if t.material_index==slot];rows.append((BVHTree.FromPolygons(vs,[list(t.vertices) for t in tris],all_triangles=True),vs,tris,texdata(color),texdata(alpha) if alpha else None,material.name))
  imported_bvh=BVHTree.FromPolygons(import_points,import_faces,all_triangles=True);forward=max(original_bvh.find_nearest(p)[3] for p in import_points);reverse=max(imported_bvh.find_nearest(p)[3] for p in original_points)
  old=np.load(source/'native-samples.npz');expected=old['expected'];h,w=expected.shape[:2];actual=np.zeros_like(expected);exhausted=0
  def sample(data,p,vs,tri):
   uv,rgba,interpolation,extension=data;v=barycentric_transform(p,*[vs[i] for i in tri.vertices],*[Vector((*uv.data[i].uv,0)) for i in tri.loops]);x,y=v.x,v.y
   if extension=='REPEAT':x%=1;y%=1
   elif extension=='CLIP' and not(0<=x<1 and 0<=y<1):return np.zeros(4)
   x=min(rgba.shape[1]-1,max(0,int(x*rgba.shape[1])));y=min(rgba.shape[0]-1,max(0,int(y*rgba.shape[0])));return rgba[y,x]
  for yy,xx in zip(*np.where(expected[:,:,3]>0)):
   initial=Vector(((930 if TREE==12 else 1070)+xx+.5,-(yy+.5)/SIN,0))+RAY*10000;accepted=[]
   for bvh,vs,tris,color,alpha,name in rows:
    origin=initial.copy()
    for step in range(256):
     p,n,f,d=bvh.ray_cast(origin,-RAY)
     if p is None:break
     tri=tris[f];a=sample(alpha,p,vs,tri)[3] if alpha else 1
     if a>=.5:accepted.append(((p-initial).length,sample(color,p,vs,tri),name));break
     origin=p-RAY*.0001
    else:exhausted+=1
   if accepted:
    _,pixel,_=min(accepted,key=lambda x:x[0]);actual[yy,xx]=np.rint(pixel*255).astype(np.uint8);actual[yy,xx,3]=255
  known=expected[:,:,3]>0;changed=known&np.any(actual[:,:,:3]!=expected[:,:,:3],axis=2);miss=known&(actual[:,:,3]==0);sheet=Image.new('RGBA',(w*3,h));sheet.paste(Image.fromarray(expected),(0,0));sheet.paste(Image.fromarray(actual),(w,0));diff=expected.copy();diff[changed]=[255,0,100,255];sheet.paste(Image.fromarray(diff),(2*w,0));sheet.resize((w*9,h*3),Image.Resampling.NEAREST).save(out/'native-comparison.png')
  write_json(out/'native-proof.json',dict(status='PASS' if not changed.any() and not miss.any() and not exhausted and max(forward,reverse)<.001 else 'HOLD',model_sha256=sha(out/'model.glb'),source_model_sha256=sha(source/'worker.blend'),known_pixels=int(known.sum()),changed_pixels=int(changed.sum()),missing_pixels=int(miss.sum()),ray_exhaustions=exhausted,export_to_approved_surface_max=forward,approved_to_export_surface_max=reverse,limits=['Analytic imported material/nearest source sampling; full production eight-view rendering remains required.','No static exclusive membership approval for shared dynamic art.']))
 finally:release()
if __name__=='__main__':main()
