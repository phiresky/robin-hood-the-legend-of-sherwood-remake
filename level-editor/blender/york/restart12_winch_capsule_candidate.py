"""Construct ordinary straight-sided oval links on the complete chain return."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-motion-physical-v2';OUT=WORK/'winch-chain-loop-prototype-v8'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector,Matrix
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
p=BASE/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(p));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();old=[o for o in scene.objects if o.name.startswith('Suspended chain')];assert old
protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in old};iron=old[0].data.materials[0]
for o in old:bpy.data.objects.remove(o,do_unlink=True)
left=world(2400.5,1059,104);right=world(2410,1064,104);direction=(right-left).normalized();radius=(right-left).length/2;mid=(left+right)/2;up=Vector((0,0,1));count=64;spacing=3.5/c;length=count*spacing;height=(length-2*math.pi*radius)/2;assert height>100
# Straight links retain the observed strand x positions and the seven-pixel
# front-link repeat. Hidden top and bottom returns are inferred geometry.
def point(t):
 t%=length
 if t<height:return left+up*t,up
 t-=height
 if t<math.pi*radius:
  a=math.pi-t/radius;return mid+up*height+direction*(radius*math.cos(a))+up*(radius*math.sin(a)),direction*math.sin(a)-up*math.cos(a)
 t-=math.pi*radius
 if t<height:return right+up*(height-t),-up
 t-=height;a=-t/radius;return mid+direction*(radius*math.cos(a))+up*(radius*math.sin(a)),direction*math.sin(a)-up*math.cos(a)
def capsule(t):
 rx,ry=1.5,3.0;arc=math.pi*rx;straight=2*(ry-rx)
 if t<arc:
  a=t/rx;return Vector((rx*math.cos(a),ry-rx+rx*math.sin(a),0)),Vector((math.cos(a),math.sin(a),0))
 t-=arc
 if t<straight:return Vector((-rx,ry-rx-t,0)),Vector((-1,0,0))
 t-=straight
 if t<arc:
  a=math.pi+t/rx;return Vector((rx*math.cos(a),-ry+rx+rx*math.sin(a),0)),Vector((math.cos(a),math.sin(a),0))
 t-=arc;return Vector((rx,-ry+rx+t,0)),Vector((1,0,0))
rows=[]
for i in range(count):
 position,tangent=point(i*spacing+5.5/c)
 # Front planes face the native art on straight runs; alternating link planes
 # turn90degrees. Curved returns continue those bases in three dimensions.
 path_normal=up.cross(direction).normalized();roll=math.atan2(path_normal.x,direction.x)*tangent.z;side=path_normal.cross(tangent)*math.cos(roll)+path_normal*math.sin(roll);side.normalize();normal=side.cross(tangent).normalized()
 if i%2:side=normal;normal=side.cross(tangent).normalized()
 rotation=Matrix((side,tangent,normal)).transposed();vertices=[];faces=[]
 for j in range(32):
  centerline,radial=capsule(j*(2*math.pi*1.5+6)/32)
  for k in range(8):
   phi=k*math.tau/8;vertices.append(centerline+radial*(.4*math.cos(phi))+Vector((0,0,.4*math.sin(phi))))
 for j in range(32):
  for k in range(8):faces.append((j*8+k,((j+1)%32)*8+k,((j+1)%32)*8+(k+1)%8,j*8+(k+1)%8))
 mesh=bpy.data.meshes.new(f'Closed oval chain link {i:03d}');mesh.from_pydata(vertices,[],faces);mesh.update();o=bpy.data.objects.new(f'Complete chain loop link {i:03d}',mesh);scene.collection.objects.link(o);o.location=position;o.rotation_euler=rotation.to_euler();o.data.materials.append(iron);o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004';o['chain_path_distance']=i*spacing;rows.append({'name':o.name,'path_distance':i*spacing,'center':list(position)})
# A complete hidden top return needs its own inferred idler and bearing shaft.
normal=direction.cross(up).normalized();top_center=mid+up*height
for name,rad,depth in [('Inferred upper chain idler',radius-2.3,2),('Inferred upper idler bearing shaft',.9,18)]:
 bpy.ops.mesh.primitive_cylinder_add(vertices=24,radius=rad,depth=depth,location=top_center);o=bpy.context.object;o.name=name;o.rotation_euler=normal.to_track_quat('Z','Y').to_euler();o.data.materials.append(iron);o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004'
bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected};OUT.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'proposal.json').write_text(json.dumps({'status':'Private inferred return prototype, not approved and not animated','source_model_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'link_count':count,'link_profile':{'centerline_half_width':1.5,'centerline_half_height':3,'circular_wire_radius':.4,'shape':'capsule'},'spacing_world':spacing,'path_length':length,'straight_height':height,'radius_world':radius,'nodes':rows,'outside_exact':len(protected),'limitations':['Visible front-link spacing and strand positions retained, but source phase fit must be rebuilt for the closed path.','The source does not reveal the upper return or prove a single-loop mechanism. This is a complete-object hypothesis to review, not established native mechanical behavior.','Bottom chain arc and drum alignment must be inspected together before retaining this hypothesis.','No chain animation or library write.']},indent=2)+'\n');print('COMPLETE CHAIN RETURN PROTOTYPE SAVED')
