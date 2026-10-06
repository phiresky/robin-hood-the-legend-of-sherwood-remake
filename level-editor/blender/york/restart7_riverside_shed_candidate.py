"""Private closed shed hypothesis from its native roof, wall and street boundaries."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-riverside-shed-v2';SOURCE=B/'grounding/york-grounded.blend';ASSET='york-riverside-storehouse-timber-shed';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def signature(o):return sha_bytes(json.dumps(dict(vertices=[list(v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],uvs=[[list(d.uv)for d in u.data]for u in o.data.uv_layers],matrix=[list(r)for r in o.matrix_world]),sort_keys=True).encode())
def sha_bytes(b):return hashlib.sha256(b).hexdigest()
def main():
 D.mkdir(exist_ok=True);assert not(D/'model.blend').exists();original=sha(SOURCE);bpy.ops.wm.open_mainfile(filepath=str(SOURCE));bpy.context.view_layer.update();collection=bpy.data.collections['york Working'];keep=[o for o in collection.all_objects if o.type=='MESH'and o.get('source_node')in [f'building-{i:03d}'for i in range(7)]+['building-086']];old=next(o for o in keep if o.get('source_node')=='building-005');context=[o for o in keep if o!=old];before={o.name:signature(o)for o in context};source_props={k:old[k]for k in old.keys()};scene=bpy.data.scenes.new('York riverside shed private');bpy.context.window.scene=scene
 protected=set(keep)
 for o in keep:
  parent=o.parent
  while parent is not None:protected.add(parent);parent=parent.parent
 for o in protected:
  scene.collection.objects.link(o);o.hide_render=False
 bpy.context.view_layer.update();assert all(signature(o)==before[o.name]for o in context)
 for o in list(bpy.data.objects):
  if o not in protected:bpy.data.objects.remove(o,do_unlink=True)
 bpy.data.objects.remove(old,do_unlink=True)
 art=Image.open(B/'baseline/covered.png').convert('RGBA');mask=Image.open(B/'baseline/masks/000001.png').convert('L');atlas=art.crop((1432,1019,1514,1139));atlas.putalpha(mask);atlas.save(D/'native-mask1-rgba.png');image=bpy.data.images.load(str(D/'native-mask1-rgba.png'),check_existing=False);image.pack()
 def material(name,observed):
  m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;n.clear();out=n.new('ShaderNodeOutputMaterial');p=n.new('ShaderNodeBsdfPrincipled');p.inputs['Base Color'].default_value=(0,0,0,1);p.inputs['Emission Color'].default_value=(.24,.24,.24,1);p.inputs['Emission Strength'].default_value=1;l.new(p.outputs['BSDF'],out.inputs['Surface']);m['source_ownership_fill']='neutral'
  if observed:
   uv=n.new('ShaderNodeUVMap');uv.uv_map='NativeSource';t=n.new('ShaderNodeTexImage');t.image=image;t.interpolation='Closest';t.extension='CLIP';l.new(uv.outputs['UV'],t.inputs['Vector']);mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.24,.24,.24,1);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(t.outputs['Color'],mix.inputs[2]);l.new(mix.outputs['Color'],p.inputs['Emission Color'])
  return m
 observed=material('Shed native observed mask1',True);unknown=material('Shed inferred gray',False)
 def mesh(name,verts,faces):
  me=bpy.data.meshes.new(name);me.from_pydata(verts,[],faces);me.update();bm=bmesh.new();bm.from_mesh(me);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(me);bm.free();o=bpy.data.objects.new(name,me);scene.collection.objects.link(o)
  for k,v in source_props.items():o[k]=v
  o['projection_component']=name.split(' / ')[-1];o['refinement_recipe']='york/restart7_riverside_shed_candidate.py';me.materials.append(observed);me.materials.append(unknown);uv=me.uv_layers.new(name='NativeSource')
  for p in me.polygons:
   p.material_index=0 if p.normal.dot(RAY)>.05 else 1
   for li in p.loop_indices:
    v=me.vertices[me.loops[li].vertex_index].co;uv.data[li].uv=((v.x-1432)/82,1-(-v.y*S-v.z*C-1019)/120)
  return o
 def solid_quad(name,top,depth):
  v=[list(p)for p in top]+[[p[0],p[1],p[2]-depth]for p in top];return mesh(name,v,[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
 def world(x,y,z):return Vector((x,(-y-C*z)/S,z))
 A=world(1478,1019,192.884);U=world(1513,1054,192.884)-A;V=world(1432,1055,170.914)-A
 normal=U.cross(V).normalized()
 def roof_z(x,y):return float(A.z-(normal.x*(x-A.x)+normal.y*(y-A.y))/normal.z)
 footprint=[(1481.93,-2059.35),(1513.80,-2117.85),(1468.0,-2142.8),(1436.15,-2084.3)];floor=109.751;top=[(x,y,roof_z(x,y)-2.0)for x,y in footprint];wall=mesh('Riverside shed / closed wall body',top+[(x,y,floor)for x,y in footprint],[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
 solid_quad('Riverside shed / roof shell',[A,A+U,A+U+V,A+V],2.0)
 for i in range(10):
  a=i/10+.004;b=(i+1)/10-.004;lift=Vector((0,0,.25+(i%3)*.12));solid_quad(f'Riverside shed / roof plank {i+1:02d}',[A+U*a+lift,A+U*b+lift,A+U*b+V+lift,A+U*a+V+lift],.55)
 # Shallow relief follows the two observed timber walls; the solid substrate
 # remains continuous and the hidden side is not reconstructed from another asset.
 for edge,count in [(1,10),(2,9)]:
  aa=Vector((*footprint[edge],0));bb=Vector((*footprint[(edge+1)%4],0));axis=bb-aa;out=Vector((axis.y,-axis.x,0)).normalized()
  # Polygon ordering is clockwise from above, hence its outward side is left.
  out=-out
  for i in range(count):
   p=aa+axis*(i/count+.004);q=aa+axis*((i+1)/count-.004);z0=floor+.05;ztp=roof_z(p.x,p.y)-2.15;ztq=roof_z(q.x,q.y)-2.15;d=.4+(i%3)*.12
   front=[Vector((p.x,p.y,z0))+out*d,Vector((q.x,q.y,z0))+out*d,Vector((q.x,q.y,ztq))+out*d,Vector((p.x,p.y,ztp))+out*d];back=[v-out*(d+.1)for v in front];mesh(f'Riverside shed / wall {edge} plank {i+1:02d}',front+back,[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
 bpy.context.view_layer.update();assert all(signature(o)==before[o.name]for o in context)
 scene['scope']='Private shed geometry only; native mask1 association inferred from artwork, not obstacle metadata. Context source surfaces retained exactly.';scene.render.engine='CYCLES';scene.cycles.samples=12;scene.render.film_transparent=True;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Shed diagnostic world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.12,.12,.12,1)
 bpy.context.preferences.filepaths.save_version=0
 for other in list(bpy.data.scenes):
  if other!=scene:bpy.data.scenes.remove(other)
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'));assert sha(SOURCE)==original
 (D/'construction.json').write_text(json.dumps(dict(source=str(SOURCE),source_sha256=original,model_sha256=sha(D/'model.blend'),asset=ASSET,context_exact=before,mask=1,mask_box=[1432,1019,82,120],native_source_sha256=sha(B/'baseline/covered.png'),native_mask_sha256=sha(B/'baseline/masks/000001.png'),floor_z=floor,roof_source_corners=[[1478,1019],[1513,1054],[1467,1090],[1432,1055]],status='Private geometry hypothesis; source coverage, ground contact and actual8 review pending',publication=False),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
