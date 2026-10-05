"""Read-only coherent planar roof/straight-return alternatives near tree47."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from scenery_geometry import Mesh
from PIL import Image

def main():
 root=OUT/'restart4-source-gaps';old=root/'shed-east-v1';e=json.loads((old/'wide-contact-v1/evidence.json').read_text());rec=next(r for r in e['inputs']if r['asset']=='croisement02-tree-47');assert sha(Path(rec['model']))==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=rec['model']);bpy.context.window.scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.view_layer.update();wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-47'and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];trees=[]
 for o in wood:
  v=[o.matrix_world@x.co for x in o.data.vertices];trees.append((o.name,v,[[int(i)for i in edge.vertices]for edge in o.data.edges],BVHTree.FromPolygons(v,[list(p.vertices)for p in o.data.polygons])))
 raw=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())['sight_obstacles'][138]['points'];p=[Vector((t['x'],-t['y']/SIN,t['z_top']/COS))for t in raw];along=p[0]-p[3];along.z=0;along.normalize();depth=p[0]-p[1];depth.z=0;slope=(p[0].z-p[1].z)/depth.length;depth.normalize();up=Vector((0,0,1));audit=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());domain=np.asarray(Image.open(old/'native-domain.png'))[:,:,3]>0;samples=[row['pixel']for n in [12,20]for row in audit['components'][n-1]['samples']];owned=[(x,y)for x,y in samples if domain[y,x]];results=[]
 for length in [16,24,30,40]:
  for fx in [1794,1798,1802]:
   for fy in [-431,-435,-439,-443]:
    back=p[0]+along*length;front=Vector((fx,fy,0));front.z=p[1].z+(front-p[1]).dot(depth)*slope
    if front.x>=back.x-1:continue
    m=Mesh()
    for j in range(8):
     a=j/8;b=(j+1)/8;q=[p[0].lerp(back,a),p[1].lerp(front,a),p[1].lerp(front,b),p[0].lerp(back,b)];start=len(m.vertices);m.vertices.extend(tuple(t-up*2)for t in q);m.vertices.extend(tuple(t)for t in q);m.faces.extend(tuple(start+i for i in f)for f in[(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])
    for a,b in [(p[1],front),(p[0],back),(front,back)]:
     axis=b-a;axis.z=0;span=axis.length;axis.normalize();side=Vector((-axis.y,axis.x,0));count=max(1,round(span/4))
     for j in range(count):q=a.lerp(b,(j+.5)/count);height=q.z-2;q.z=height/2;m.box(q,axis,side,span/count+.03,1.5,height)
    for q in [front,back]:m.tube(Vector((q.x,q.y,0)),q,2.2,n=8)
    v=[Vector(x)for x in m.vertices];tree=BVHTree.FromPolygons(v,m.faces);edges=set(tuple(sorted((f[i],f[(i+1)%len(f)])))for f in m.faces for i in range(len(f)));hits=[]
    for name,ov,oe,ot in trees:
     count=0
     for a,b in oe:
      delta=ov[b]-ov[a]
      if delta.length<1e-6:continue
      q,n,index,d=tree.ray_cast(ov[a],delta.normalized(),delta.length)
      if q is not None and 1e-5<d<delta.length-1e-5:count+=1
     for a,b in edges:
      delta=v[b]-v[a]
      if delta.length<1e-6:continue
      q,n,index,d=ot.ray_cast(v[a],delta.normalized(),delta.length)
      if q is not None and 1e-5<d<delta.length-1e-5:count+=1
     hits.append(dict(object=name,intersections=count))
    covered=0;missing=[]
    for x,y in owned:
     point,n,index,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
     if point is not None:covered+=1
     else:missing.append([x,y])
    row=dict(back_extension_length=length,front=list(front),back=list(back),owned_target=len(owned),covered=covered,missing=missing,intersections=sum(r['intersections']for r in hits),wood=hits);results.append(row);print('LAYOUT',length,fx,fy,covered,row['intersections'],flush=True)
 out=root/'shed-layout-research-v1';out.mkdir(exist_ok=False);write_json(out/'report.json',dict(status='Private planar roof/straight return alternatives; no saved geometry or render',tree_model_sha256=rec['model_sha256'],original_roof_pitch_preserved=True,original_front_untouched=True,candidates=results,ranking=sorted(range(len(results)),key=lambda i:(results[i]['intersections'],405-results[i]['covered']))))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
