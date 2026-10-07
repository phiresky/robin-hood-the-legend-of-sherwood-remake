"""Measure a winding return around the retained shaft, without changing wood."""
import json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-components-source-v2';OUT=WORK/'winch-shaft-return-study-v2'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
def world(x,y,z):return Vector((x,-y/s,z/c))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));
def station(x):return center+axis*((x-center.x)/axis.x)
def capsule(t,rx,ry=3):
 arc=math.pi*rx;straight=2*(ry-rx)
 if t<arc:
  a=t/rx;return Vector((rx*math.cos(a),ry-rx+rx*math.sin(a),0)),Vector((math.cos(a),math.sin(a),0))
 t-=arc
 if t<straight:return Vector((-rx,ry-rx-t,0)),Vector((-1,0,0))
 t-=straight
 if t<arc:
  a=math.pi+t/rx;return Vector((rx*math.cos(a),-ry+rx+rx*math.sin(a),0)),Vector((math.cos(a),math.sin(a),0))
 t-=arc;return Vector((rx,-ry+rx+t,0)),Vector((1,0,0))
def route(radius,left_x,count=72):
 a=station(left_x+radius*radial.x);b=station(2410-radius*radial.x)
 def curve(u,upper):
  smooth=u*u*(3-2*u);d=6*u*(1-u)
  if upper:return a+(b-a)*smooth+radius*(-math.cos(math.pi*u)*radial+math.sin(math.pi*u)*up),(b-a)*d+radius*math.pi*(math.sin(math.pi*u)*radial+math.cos(math.pi*u)*up)
  return b+(a-b)*smooth+radius*(math.cos(math.pi*u)*radial-math.sin(math.pi*u)*up),(a-b)*d-radius*math.pi*(math.sin(math.pi*u)*radial+math.cos(math.pi*u)*up)
 us=np.linspace(0,1,1025);points=[curve(float(u),True)[0] for u in us];dist=np.r_[0,np.cumsum([(q-p).length for p,q in zip(points,points[1:])])];arc=float(dist[-1]);length=count*spacing;height=(length-2*arc)/2;left=a-radius*radial;right=b+radius*radial
 def pose(t,index):
  t%=length
  if t<height:point,tangent=left+up*t,up
  elif t<height+arc:
   u=float(np.interp(t-height,dist,us));point,tangent=curve(u,True);point+=up*height
  elif t<2*height+arc:point,tangent=right+up*(2*height+arc-t),-up
  else:u=float(np.interp(t-2*height-arc,dist,us));point,tangent=curve(u,False)
  tangent.normalize();side=(screen_side-tangent*tangent.dot(screen_side)).normalized();normal=side.cross(tangent).normalized()
  if index%2:side=normal;normal=side.cross(tangent).normalized()
  return point,Matrix((side,tangent,normal)).transposed()
 return pose,{'radius':radius,'left_source_x':left_x,'right_source_x':2410,'left_station':list(a),'right_station':list(b),'straight_height':height,'return_length':arc,'path_length':length,'link_count':count}
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();scope=set(json.loads((BASE/'component-freeze.json').read_text())['scope']);vertices=[];faces=[];owners=[]
for o in scene.objects:
 if o.name not in scope:continue
 off=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);o.data.calc_loop_triangles()
 for f in o.data.loop_triangles:faces.append(tuple(off+i for i in f.vertices));owners.append(o.name)
body=BVHTree.FromPolygons(vertices,faces,all_triangles=True);rows=[]
for radius in (8.6,8.8,9,9.2,9.4):
 for left_x in (2400,2400.5,2401,2401.5):
  pose,params=route(radius,left_x)
  for rx in (1.2,):
   local=[];surface=[];perimeter=2*math.pi*rx+4*(3-rx)
   for j in range(32):
    p,n=capsule(j*perimeter/32,rx)
    for k in range(8):phi=k*math.tau/8;local.append(p+n*(.3*math.cos(phi))+Vector((0,0,.3*math.sin(phi))))
   for j in range(32):
    for k in range(8):surface.append((j*8+k,((j+1)%32)*8+k,((j+1)%32)*8+(k+1)%8,j*8+(k+1)%8))
   vv=[];ff=[]
   for i in range(72):
    p,r=pose(i*spacing+5.5/c,i);off=len(vv);vv.extend(p+r@v for v in local);ff.extend(tuple(off+j for j in f) for f in surface)
   chain=BVHTree.FromPolygons(vv,ff);overlaps=chain.overlap(body);counts={}
   for _,f in overlaps:counts[owners[f]]=counts.get(owners[f],0)+1
   rows.append({**params,'capsule_half_width':rx,'wire_radius':.3,'minimum_chain_game_z':min(v.z*c for v in vv),'body_intersection_pairs':counts,'pair_count':len(overlaps)})
rows.sort(key=lambda r:r['pair_count']);OUT.mkdir();(OUT/'study.json').write_text(json.dumps({'status':'Private shaft-aligned helical return diagnostic, no candidate saved','rows':rows},indent=2)+'\n');print(json.dumps(rows[:6],indent=2))
