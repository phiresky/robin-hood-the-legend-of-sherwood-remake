"""Check the small wall-toe overlap without changing either approved receiver."""
from pathlib import Path
import sys,json,hashlib
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from leaf_state_scene_context import load_scene
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def tree(obj):
 obj.data.calc_loop_triangles();points=[obj.matrix_world@v.co for v in obj.data.vertices];triangles=[tuple(t.vertices)for t in obj.data.loop_triangles]
 return BVHTree.FromPolygons(points,triangles,all_triangles=True),points,triangles

def main():
 root=OUT/'restart9-hiding-scatter';out=root/'wall-toe-contact-v1';out.mkdir(exist_ok=False);worker=root/'mound-support-variants-v2';report=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==report['model_sha256'];instance='mission-Tac02_FoB_EC-patch-022';row=next(r for r in report['records']if instance in r['instances']);scene,static,pins,base=load_scene()
 with bpy.data.libraries.load(str(model),link=False)as(src,dst):dst.objects=[row['object']]
 mound=dst.objects[0];scene.collection.objects.link(mound);bpy.context.view_layer.update();mt,mp,mtris=tree(mound);walls=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];assert walls
 overlaps=[]
 for wall in walls:
  wt,wp,wtris=tree(wall);pairs=mt.overlap(wt);overlaps.append(dict(object=wall.name,triangle_pairs=len(pairs),pairs=pairs[:100],matrix_world=[list(r)for r in wall.matrix_world]))
 audit=json.loads((root/'scene-receivers-v1/report.json').read_text());source=next(r for r in audit['records']if r['instance']==instance and r['state']=='initial');samples=[]
 for s in source['samples']:
  if s['receiver']!='croisement02-southeast-stone-wall-and-gate':continue
  x,y=s['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000;hit,normal,index,d=mt.ray_cast(origin,-RAY);wallpoint=Vector(s['point']);samples.append(dict(pixel=s['pixel'],wall=s['point'],mound=list(hit)if hit else None,source_ray_separation=(hit-wallpoint).dot(RAY)if hit else None))
 result=dict(status='HOLD'if any(r['triangle_pairs']for r in overlaps)else 'PASS',instance=instance,model_sha256=sha(model),base_sha256=sha(base),substitutions=pins,wall_triangle_intersections=overlaps,native_wall_samples=samples,scope='Geometric wall-toe triangle intersection only; high canopy occlusion excluded from support inference. No geometry changes.')
 (out/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
