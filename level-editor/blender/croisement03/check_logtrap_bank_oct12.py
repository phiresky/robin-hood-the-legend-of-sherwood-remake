"""Read-only contact qualification against a separately pinned private bank."""
from pathlib import Path
import sys, json, hashlib, shutil
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 acquire(slots=2)
 try:
  mem=dict(x.split(':',1) for x in Path('/proc/meminfo').read_text().splitlines());assert int(mem['MemAvailable'].split()[0])*1024>=6*1024**3;assert shutil.disk_usage(ROOT).free>=10*1024**3
  target=OUT/'bank-contact-v2';target.mkdir(exist_ok=False)
  model=OUT/'owned-candidate-v2/worker.blend';bank=ROOT/'level-editor/work/croisement03-refinement/restart2/bank-coupled-left-full-source-v3/worker.blend'
  assert sha(model)=='b910b45b5b1d759a7db09c65f9d0eb7b0e6a4299f5b7b9afe668acad10e95283'
  assert sha(bank)=='cf1a1347f706ef9e46d2cfc2b391424bed21ea59a9af61509958cbb7cf163f6b'
  bpy.ops.wm.open_mainfile(filepath=str(model));logs=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('state_endpoint')]
  with bpy.data.libraries.load(str(bank),link=False) as (src,dst):dst.objects=list(src.objects)
  imported=[]
  for o in dst.objects:
   if o is not None:bpy.context.scene.collection.objects.link(o);imported.append(o)
  bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get()
  def mesh(o):
   e=o.evaluated_get(deps);m=e.to_mesh();m.calc_loop_triangles();v=[e.matrix_world@x.co for x in m.vertices];t=[tuple(x.vertices) for x in m.loop_triangles];e.to_mesh_clear();return v,t
  banks=[]
  for o in imported:
   if o.type!='MESH':continue
   v,t=mesh(o)
   if not t:continue
   banks.append((o,v,BVHTree.FromPolygons(v,t,all_triangles=True)))
  rows=[]
  for o in logs:
   v,t=mesh(o);tree=BVHTree.FromPolygons(v,t,all_triangles=True);contacts=[]
   for ob,bv,bt in banks:
    overlaps=tree.overlap(bt);heights=[]
    for p in v:
     hit,normal,index,distance=bt.ray_cast(Vector((p.x,p.y,10000)),Vector((0,0,-1)),20000)
     if hit is not None:heights.append({'vertex_z':p.z,'receiver_z':hit.z,'delta':p.z-hit.z})
    if overlaps or heights:contacts.append({'receiver':ob.name,'native_obstacle':ob.get('native_obstacle'),'triangle_overlap_pairs':len(overlaps),'vertical_samples':len(heights),'minimum_vertex_delta':min((x['delta'] for x in heights),default=None),'maximum_receiver_z':max((x['receiver_z'] for x in heights),default=None)})
   rows.append({'object':o.name,'state':o['state_endpoint'],'minimum_z':min(p.z for p in v),'receivers':contacts})
  report={'status':'DIAGNOSTIC_PRIVATE_BANK_HOLD','model_sha256':sha(model),'bank_sha256':sha(bank),'bank_meshes':[o.name for o,_,_ in banks],'rows':rows,'limitations':['Private bank is unapproved context, not canonical support authority.','Triangle intersections and vertical vertex rays are diagnostics; no contact approval or pixel ownership is implied.','No source model saved or altered.']}
  (target/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)
 finally:release()
if __name__=='__main__':main()
