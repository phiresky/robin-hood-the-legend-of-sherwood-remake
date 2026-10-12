"""Join the intersecting inferred basal stems while preserving all measured bark."""
import json,sys,hashlib,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'));from render_slots import acquire,release
B=R/'level-editor/work/active/croisement03/restart2';I=B/'tree01-cpu-upper-v1';O=B/'tree01-cpu-root-union-v1'
def resources():
 assert shutil.disk_usage(R).free>10*1024**3
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def main():
 resources();acquire(slots=2)
 try:
  resources();assert not O.exists();data=json.loads((I/'geometry.json').read_text());assert json.loads((I/'cpu-firsthits.json').read_text())['status']=='PASS_CPU_PROPOSED_BARK_SEEDS';bpy.ops.wm.read_factory_settings(use_empty=True);parts={p['source_obstacle_context']:p for p in data['parts']};objects={};known=[]
  for id in [4,5]:
   p=parts[id];m=bpy.data.meshes.new(str(id));m.from_pydata(p['vertices'],[],p['faces']);m.update();o=bpy.data.objects.new(str(id),m);bpy.context.scene.collection.objects.link(o);objects[id]=o;r=p['vertices_per_ring']
   for j in p['observed_section_indices']+p['proposed_upper_sections']:known.extend(p['vertices'][j*r:(j+1)*r])
  a,b=objects[4],objects[5];bpy.context.view_layer.objects.active=a;mod=a.modifiers.new('Coherent inferred basal junction','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=b;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(b,do_unlink=True);m=a.data;bm=bmesh.new();bm.from_mesh(m);assert all(e.is_manifold for e in bm.edges);assert bm.calc_volume()>0;bm.free();m.calc_loop_triangles();vertices=[list(v.co)for v in m.vertices];faces=[list(t.vertices)for t in m.loop_triangles];saved=np.array(vertices);errors=[float(np.min(np.linalg.norm(saved-v,axis=1)))for v in np.array(known)];assert max(errors)<1e-4
  combined={'source_obstacle_context':4,'source_contexts':[4,5],'vertices':vertices,'faces':faces,'source_ownership_assigned':False,'construction':'Exact Boolean union restricted to intersecting inferred lower stems; source bark sections retained.','measured_bark_vertex_max_error':max(errors),'native_upper_constraints_retained':True,'branch_section_width_depth_ratio':1.0}
  data['parts']=[parts[3],combined];data['root_union']={'input_sha256':hashlib.sha256((I/'geometry.json').read_bytes()).hexdigest(),'checked_source_vertices':len(known),'max_source_vertex_error':max(errors),'limits':['Private inferred root connection, not an observed bifurcation.','Fourth left stem and full foliage remain unresolved.','Independent saved native firsthits and eight-view self-review required.']};O.mkdir();(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print(data['root_union'])
 finally:release()
if __name__=='__main__':main()
