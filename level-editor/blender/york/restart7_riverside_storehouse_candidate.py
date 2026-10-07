"""Private riverside stone storehouse: closed masonry, fitted hip roof and recessed openings."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_review import _tree
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-riverside-storehouse-v2';SOURCE=B/'restart7-riverside-shed-v2/model.blend';ASSET='york-riverside-stone-storehouse';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def signature(o):return sha_bytes(json.dumps(dict(vertices=[list(v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],uvs=[[list(d.uv)for d in u.data]for u in o.data.uv_layers],matrix=[list(r)for r in o.matrix_world]),sort_keys=True).encode())
def sha_bytes(b):return hashlib.sha256(b).hexdigest()
def main():
 D.mkdir(exist_ok=True);assert not(D/'model.blend').exists();original=sha(SOURCE);bpy.ops.wm.open_mainfile(filepath=str(SOURCE));bpy.context.view_layer.update();keep=[o for o in bpy.context.scene.objects if o.type=='MESH'];oldset=[o for o in keep if o.get('asset_group')==ASSET];old=next(o for o in oldset if o.get('source_node')=='building-000');context=[o for o in keep if o not in oldset];before={o.name:signature(o)for o in context};source_props={k:old[k]for k in old.keys()};scene=bpy.data.scenes.new('York riverside storehouse private');bpy.context.window.scene=scene
 protected=set(keep)
 for o in keep:
  parent=o.parent
  while parent is not None:protected.add(parent);parent=parent.parent
 for o in protected:
  scene.collection.objects.link(o);o.hide_render=False
 bpy.context.view_layer.update();assert all(signature(o)==before[o.name]for o in context)
 for o in list(bpy.data.objects):
  if o not in protected:bpy.data.objects.remove(o,do_unlink=True)
 for o in oldset:bpy.data.objects.remove(o,do_unlink=True)
 art=Image.open(B/'baseline/covered.png').convert('RGBA');mask=Image.open(B/'baseline/masks/000000.png').convert('L');atlas=art.crop((1449,840,1645,1132));atlas.putalpha(mask);atlas.save(D/'native-mask0-rgba.png');image=bpy.data.images.load(str(D/'native-mask0-rgba.png'),check_existing=False);image.pack()
 def material(name,observed,source_image=None):
  m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;n.clear();out=n.new('ShaderNodeOutputMaterial');p=n.new('ShaderNodeBsdfPrincipled');p.inputs['Base Color'].default_value=(0,0,0,1);p.inputs['Emission Color'].default_value=(.24,.24,.24,1);p.inputs['Emission Strength'].default_value=1;l.new(p.outputs['BSDF'],out.inputs['Surface']);m['source_ownership_fill']='neutral'
  if observed:
   uv=n.new('ShaderNodeUVMap');uv.uv_map='NativeSource';t=n.new('ShaderNodeTexImage');t.image=source_image or image;t.interpolation='Closest';t.extension='CLIP';l.new(uv.outputs['UV'],t.inputs['Vector']);mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.24,.24,.24,1);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(t.outputs['Color'],mix.inputs[2]);l.new(mix.outputs['Color'],p.inputs['Emission Color'])
  return m
 observed=material('Storehouse native observed mask0',True);unknown=material('Storehouse inferred gray',False)
 def mesh(name,verts,faces):
  me=bpy.data.meshes.new(name);me.from_pydata(verts,[],faces);me.update();bm=bmesh.new();bm.from_mesh(me);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(me);bm.free();o=bpy.data.objects.new(name,me);scene.collection.objects.link(o)
  for k,v in source_props.items():o[k]=v
  o['projection_component']=name.split(' / ')[-1];o['refinement_recipe']='york/restart7_riverside_storehouse_candidate.py';me.materials.append(observed);me.materials.append(unknown);uv=me.uv_layers.new(name='NativeSource')
  for p in me.polygons:
   p.material_index=0 if p.normal.dot(RAY)>.05 else 1
   for li in p.loop_indices:
    v=me.vertices[me.loops[li].vertex_index].co;uv.data[li].uv=((v.x-1449)/196,1-(-v.y*S-v.z*C-840)/292)
  return o
 def solid_quad(name,top,depth):
  v=[list(p)for p in top]+[[p[0],p[1],p[2]-depth]for p in top];return mesh(name,v,[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
 def world(x,y,z):return Vector((x,(-y-C*z)/S,z))
 floor=109.8713
 footprint=[(1468.80,-2031.25),(1577.5,-1975.5),(1626.8,-2071.7),(1518.05,-2127.4)]
 top=[(1466.8,-2031.25,302.0),(1577.5,-1975.5,302.0),(1631.8,-2071.7,302.0),(1518.05,-2127.4,302.0)]
 body=mesh('Riverside storehouse / closed masonry',top+[(x,y,floor)for x,y in footprint],[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
 def assign(o,index):o['source_node']=f'building-{index:03d}';o['source_obstacle']=f'building-{index:03d}'
 assign(body,0)
 def recess(points,edge,depth,label):
  p0=Vector(top[edge]);p1=Vector(top[(edge+1)%4]);bottom=Vector((*footprint[edge],floor));n=(p1-p0).cross(bottom-p0).normalized();front=[]
  for x,y in points:
   start=Vector((x,-y/S,0));front.append(start+RAY*((p0-start).dot(n)/RAY.dot(n)))
  count=len(front);verts=[p+n*2 for p in front]+[p-n*depth for p in front];faces=[tuple(range(count)),tuple(range(2*count-1,count-1,-1))]+[(i,(i+1)%count,(i+1)%count+count,i+count)for i in range(count)]
  cutter=mesh('Temporary opening '+label,verts,faces);mod=body.modifiers.new('Native opening '+label,'BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter;bpy.context.view_layer.objects.active=body;body.select_set(True);bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(cutter,do_unlink=True)
 # Openings follow the dark interior pixels; their small unseen depth is inferred.
 recess([(1536,977),(1549,973),(1549,989),(1536,993)],2,5,'upper front left')
 recess([(1567,965),(1579,961),(1579,980),(1567,984)],2,5,'upper front middle')
 recess([(1607,953),(1619,949),(1619,970),(1607,974)],2,5,'upper front right')
 recess([(1581,1008),(1585,1005),(1586,1024),(1581,1021)],2,4,'front arrow slit')
 recess([(1538,1119),(1558,1114),(1557,1081),(1551,1076),(1543,1077),(1538,1084)],2,4,'arched door')
 recess([(1475,945),(1481,952),(1481,965),(1475,958)],3,4,'upper side left')
 recess([(1505,973),(1511,980),(1511,989),(1505,982)],3,4,'upper side right')
 # Restore direct native projection on new Boolean jamb and back surfaces.
 uv=body.data.uv_layers.get('NativeSource')
 for p in body.data.polygons:
  p.material_index=0 if p.normal.dot(RAY)>.05 else 1
  for li in p.loop_indices:
   v=body.data.vertices[body.data.loops[li].vertex_index].co;uv.data[li].uv=((v.x-1449)/196,1-(-v.y*S-v.z*C-840)/292)
 A=Vector((1453.5,-2033.0,300.3));B0=Vector((1577.6,-1960.6,300.3));C0=Vector((1641.5,-2069.6,300.3));E=Vector((1511.65,-2138.25,300.3));L=Vector((1519,-2062,396.5));R=Vector((1571.5,-2034.8,398.0))
 def roofpiece(name,points,index):
  n=len(points);v=list(points)+[p-Vector((0,0,2.5))for p in points];faces=[tuple(range(n)),tuple(range(2*n-1,n-1,-1))]+[(i,(i+1)%n,(i+1)%n+n,i+n)for i in range(n)];o=mesh(name,v,faces);assign(o,index);return o
 roofpiece('Riverside storehouse / front roof shell',[E,C0,R,L],1);roofpiece('Riverside storehouse / left hip shell',[A,E,L],2);roofpiece('Riverside storehouse / rear roof shell',[B0,A,L,R],3);roofpiece('Riverside storehouse / right hip shell',[C0,B0,R],4)
 n=(C0-E).cross(R-E).normalized()
 def roof_z(x,y):return E.z-(n.x*(x-E.x)+n.y*(y-E.y))/n.z
 chimney_xy=[(1569.4,-2069.5),(1581.95,-2062.7),(1587.35,-2072.72),(1574.82,-2079.52)];low=min(roof_z(x,y)for x,y in chimney_xy)-2;ct=[(x,y,381.0)for x,y in chimney_xy];chimney=mesh('Riverside storehouse / closed chimney',ct+[(x,y,low)for x,y in chimney_xy],[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)]);assign(chimney,6)
 bpy.context.view_layer.update();assert all(signature(o)==before[o.name]for o in context)
 # Keep each observed atlas on its physical first-hit receiver. The audit domain
 # remains the independent original mask, including delegated and missing rays.
 own=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==ASSET]
 tree,owners,_=_tree(own+context);domain=np.array(mask)>0;alphas={o:np.zeros(domain.shape,dtype=np.uint8)for o in own};foreign={};missing=[]
 for yy,xx in np.argwhere(domain):
  x=int(xx)+1449;y=int(yy)+840;hit,_,idx,_=tree.ray_cast(Vector((x+.5,-(y+.5)/S,0))+RAY*5000,-RAY)
  if hit is None:missing.append([x,y])
  elif owners[idx]in alphas:alphas[owners[idx]][yy,xx]=np.array(mask)[yy,xx]
  else:foreign.setdefault(owners[idx].name,[]).append([x,y])
 ownership={}
 for o,a in alphas.items():
  path=D/(o['source_node']+'-owned-rgba.png');owned=art.crop((1449,840,1645,1132));owned.putalpha(Image.fromarray(a));owned.save(path);tex=bpy.data.images.load(str(path),check_existing=False);tex.pack();o.data.materials[0]=material(o['source_node']+' native owned source',True,tex);ownership[o.name]=dict(accepted_pixels=int((a>0).sum()),atlas_sha256=sha(path))
 (D/'projection-ownership.json').write_text(json.dumps(dict(domain_pixels=int(domain.sum()),owners=ownership,foreign=foreign,missing=missing,rule='Native first-hit per receiver; original mask0 remains independent audit domain; neighboring shed source is unknown on hidden masonry.'),indent=2)+'\n')
 scene['scope']='Private storehouse geometry only; native mask0 association inferred from artwork/depth. Frozen shed and terrain context retained exactly; Building55 door metadata unchanged.';scene.render.engine='CYCLES';scene.cycles.samples=12;scene.render.film_transparent=True;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Shed diagnostic world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.12,.12,.12,1)
 bpy.context.preferences.filepaths.save_version=0
 for other in list(bpy.data.scenes):
  if other!=scene:bpy.data.scenes.remove(other)
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'));assert sha(SOURCE)==original
 (D/'construction.json').write_text(json.dumps(dict(source=str(SOURCE),source_sha256=original,model_sha256=sha(D/'model.blend'),asset=ASSET,context_exact=before,mask=0,mask_box=[1449,840,196,292],native_source_sha256=sha(B/'baseline/covered.png'),native_mask_sha256=sha(B/'baseline/masks/000000.png'),floor_z=floor,roof_shell_parts=[1,2,3,4],door_building_index=55,status='Private geometry hypothesis; source coverage, ground contact and actual8 review pending',publication=False),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
