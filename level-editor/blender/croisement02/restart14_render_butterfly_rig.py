"""Inspect conserved butterfly anatomy under three explicit rigid hinge poses."""
from pathlib import Path
import sys,json,math,hashlib,shutil
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies/rig-v1';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));B=np.array([[1,0,0],[0,-SIN,-COS],[0,-COS,SIN]])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def rot(axis,angle):
 a=math.radians(angle);c=math.cos(a);s=math.sin(a)
 return {'x':np.array([[1,0,0],[0,c,-s],[0,s,c]]),'y':np.array([[c,0,s],[0,1,0],[-s,0,c]]),'z':np.array([[c,-s,0],[s,c,0],[0,0,1]])}[axis]
def mesh(name,verts,faces):
 m=bpy.data.meshes.new(name);m.from_pydata([tuple(B@np.array(v)) for v in verts],[],[list(reversed(f)) for f in faces]);m.update();return m

def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 shutil.copyfile(__file__,OUT/'executed-recipe.py');p=json.loads((OUT/'fit.json').read_text());bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.engine='CYCLES';s.cycles.samples=12;s.cycles.use_denoising=False;s.render.threads_mode='FIXED';s.render.threads=4;s.render.resolution_x=256;s.render.resolution_y=256;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.world=bpy.data.worlds.new('Review');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
 outline=p['wing_outline'];n=len(outline);front=[[2,-.3,.19]]+outline;back=[[x,y,z-p['wing_thickness']] for x,y,z in front];verts=front+back;faces=[]
 for i in range(n):a=1+i;b=1+(i+1)%n;faces.extend([[0,a,b],[n+1,n+1+b,n+1+a],[a,n+1+a,n+1+b,b]])
 meshes={}
 for sign,name in [(-1,'left'),(1,'right')]:meshes[name]=mesh(name,[[sign*x,y,z] for x,y,z in verts],faces if sign==1 else [list(reversed(f)) for f in faces])
 bv=[];bf=[];rings=12;segments=24
 bv.append([0,4.5,0])
 for k in range(1,rings):
  a=math.pi*k/rings
  for j in range(segments):t=math.tau*j/segments;bv.append([.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)])
 bv.append([0,-4.5,0]);end=len(bv)-1
 for j in range(segments):bf.append([0,1+(j+1)%segments,1+j]);bf.append([end,1+(rings-2)*segments+j,1+(rings-2)*segments+(j+1)%segments])
 for k in range(rings-2):
  for j in range(segments):a=1+k*segments+j;b=1+k*segments+(j+1)%segments;bf.append([a,b,b+segments,a+segments])
 meshes['body']=mesh('body',bv,bf)
 for poly in meshes['body'].polygons:poly.use_smooth=True
 def material(row):
  mat=bpy.data.materials.new(f"Own source phase{row['phase']}");mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links;geo=nodes.new('ShaderNodeNewGeometry');image=nodes.new('ShaderNodeTexImage');image.image=bpy.data.images.load(row['texture']);image.image.pack();image.interpolation='Closest';image.extension='EXTEND';combine=nodes.new('ShaderNodeCombineXYZ');bbox=row['source']['bbox']
  for axis,vec,offset,denom,invert in [('X',(1,0,0),bbox[0],bbox[2],False),('Y',(0,-SIN,-COS),bbox[1],bbox[3],True)]:
   dot=nodes.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=vec;links.new(geo.outputs['Position'],dot.inputs[0]);sub=nodes.new('ShaderNodeMath');sub.operation='SUBTRACT';sub.inputs[1].default_value=offset;links.new(dot.outputs['Value'],sub.inputs[0]);div=nodes.new('ShaderNodeMath');div.operation='DIVIDE';div.inputs[1].default_value=denom;links.new(sub.outputs[0],div.inputs[0]);result=div.outputs[0]
   if invert:inv=nodes.new('ShaderNodeMath');inv.operation='SUBTRACT';inv.inputs[0].default_value=1;links.new(result,inv.inputs[1]);result=inv.outputs[0]
   links.new(result,combine.inputs[axis])
  links.new(combine.outputs[0],image.inputs['Vector']);em=nodes.new('ShaderNodeEmission');links.new(image.outputs['Color'],em.inputs['Color']);output=nodes.new('ShaderNodeOutputMaterial');links.new(em.outputs[0],output.inputs[0]);return mat
 rigs=[]
 for row in p['poses']:
  parent=bpy.data.objects.new(f"Butterfly conserved rig phase{row['phase']}",None);s.collection.objects.link(parent);rx,ry,rz,left,right,dx,dy=row['parameters'];rotation=rot('z',rz)@rot('y',ry)@rot('x',rx);parent.rotation_mode='QUATERNION';parent.rotation_quaternion=Matrix((B@rotation@B.T).tolist()).to_quaternion();sx=row['source']['bbox'][0]+row['source_center'][0]+dx;sy=row['source']['bbox'][1]+row['source_center'][1]+dy;z=row['inferred_altitude'];parent.location=(sx,-(sy+COS*z)/SIN,z);parent['phase']=row['phase'];parent['inferred_altitude']=z;mat=material(row)
  children=[]
  for name in ['body','left','right']:
   ob=bpy.data.objects.new(name+f" phase{row['phase']}",meshes[name]);s.collection.objects.link(ob);ob.parent=parent
   if not ob.data.materials:ob.data.materials.append(mat)
   ob.material_slots[0].link='OBJECT';ob.material_slots[0].material=mat
   if name!='body':sign=-1 if name=='left' else 1;angle=left if name=='left' else right;ob.location=Vector(B@np.array([sign*.30,0,0]));ob.rotation_mode='QUATERNION';ob.rotation_quaternion=Matrix((B@rot('y',-sign*angle)@B.T).tolist()).to_quaternion();ob['hinge_degrees']=angle
   children.append(ob)
  rigs.append((row,parent,children))
 camera=bpy.data.cameras.new('Native-first review camera');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam
 lights=[]
 for name,offset,energy in [('key',(-25,-35,50),22000),('fill',(35,10,20),9000)]:
  data=bpy.data.lights.new(name,'AREA');data.energy=energy;data.size=25;ob=bpy.data.objects.new(name,data);s.collection.objects.link(ob);lights.append((ob,Vector(offset)))
 for row,parent,children in rigs:
  for ob in children:ob.hide_render=row['phase']!=0;ob.hide_viewport=row['phase']!=0
 bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
 # Reopen immutable candidate for every subsequent review image.
 bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));s=bpy.context.scene;cam=s.camera;rigs=[(row,bpy.data.objects[f"Butterfly conserved rig phase{row['phase']}"], [bpy.data.objects[name+f" phase{row['phase']}"] for name in ['body','left','right']]) for row in p['poses']];lights=[(bpy.data.objects['key'],Vector((-25,-35,50))),(bpy.data.objects['fill'],Vector((35,10,20)))]
 solid=bpy.data.materials.new('Solid inspection');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.40,.38,.30,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.8
 for row,parent,children in rigs:
  for other,_,obs in rigs:
   for ob in obs:ob.hide_render=other['phase']!=row['phase'];ob.hide_viewport=other['phase']!=row['phase']
  center=parent.location
  for light,offset in lights:light.location=center+offset;light.rotation_euler=(-offset).to_track_quat('-Z','Y').to_euler()
  for mode in ['actual','solid']:
   folder=OUT/f"phase-{row['phase']:02d}"/mode;folder.mkdir(parents=True,exist_ok=True);s.view_layers[0].material_override=solid if mode=='solid' else None
   for i in range(8):
    angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();s.render.filepath=str(folder/f'view-{i}.png');bpy.ops.render.render(write_still=True)
   sheet=Image.new('RGB',(1024,512),'#252525')
   for i in range(8):im=Image.open(folder/f'view-{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
   sheet.save(folder/'sheet.png')
 # Conserved local geometry is physically shared, not merely similar.
 guard={'model_sha256':sha(OUT/'model.blend'),'body_mesh_shared':len({id(obs[0].data) for _,_,obs in rigs})==1,'left_mesh_shared':len({id(obs[1].data) for _,_,obs in rigs})==1,'right_mesh_shared':len({id(obs[2].data) for _,_,obs in rigs})==1,'pose_changes':'Rigid parent orientation and left/right hinge rotations only. No per-phase vertex morphs or changing topology.','native_first':True,'source_fit':p['poses'],'source_cycle_ticks':198,'full_sequence_status':'Only phases0/2/18 are fitted; remaining poses not yet represented.'}
 assert guard['body_mesh_shared'] and guard['left_mesh_shared'] and guard['right_mesh_shared'];(OUT/'validation.json').write_text(json.dumps(guard,indent=2)+'\n');print('RIG_READY',guard['model_sha256'])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
