"""Join Tree01's inferred basal connection and source-traced fourth branch."""
import json,sys,hashlib,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'));from render_slots import acquire,release
B=R/'level-editor/work/active/croisement03/restart2';I=B/'tree01-cpu-upper-v2';BR=B/'tree01-left-branch-source-v2';O=B/'tree01-cpu-complete-wood-v3'
def resources():
 assert shutil.disk_usage(R).free>10*1024**3+32*1024**2
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def union(p,q,contexts):
 objects=[];known=[];volumes=[]
 for part in [p,q]:
  m=bpy.data.meshes.new(str(part['source_obstacle_context']));m.from_pydata(part['vertices'],[],part['faces']);m.update();o=bpy.data.objects.new(m.name,m);bpy.context.scene.collection.objects.link(o);objects.append(o);bm=bmesh.new();bm.from_mesh(m);volumes.append(bm.calc_volume());bm.free();r=part['vertices_per_ring']
  for j in part['observed_section_indices']+part['proposed_upper_sections']:known.extend(part['vertices'][j*r:(j+1)*r])
 a,b=objects;bpy.context.view_layer.objects.active=a;mod=a.modifiers.new('Coherent hidden wood junction','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=b;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(b,do_unlink=True);m=a.data;bm=bmesh.new();bm.from_mesh(m);assert all(e.is_manifold for e in bm.edges)and all(f.calc_area()>1e-8 for f in bm.faces);volume=bm.calc_volume();assert max(volumes)-1e-3<=volume<=sum(volumes)+1e-3
 unseen=set(bm.verts);components=0
 while unseen:
  components+=1;stack=[unseen.pop()]
  while stack:
   for e in stack.pop().link_edges:
    for v in e.verts:
     if v in unseen:unseen.remove(v);stack.append(v)
 assert components==1,('Disconnected union',contexts,components);bm.free();m.calc_loop_triangles();vertices=[list(v.co)for v in m.vertices];faces=[list(t.vertices)for t in m.loop_triangles];saved=np.array(vertices);errors=[float(np.min(np.linalg.norm(saved-v,axis=1)))for v in np.array(known)];assert max(errors)<1e-4,('Measured bark moved',contexts,max(errors),[list(v)for v,e in zip(known,errors)if e>=1e-4])
 return {'source_obstacle_context':p['source_obstacle_context'],'source_contexts':contexts,'vertices':vertices,'faces':faces,'source_ownership_assigned':False,'construction':'Exact union of source-preserving wood with inferred hidden junctions.','measured_bark_vertex_max_error':max(errors),'preserved_measured_vertices':len(known),'branch_section_width_depth_ratio':1.0,'closed_connected_components':components,'volume':volume,'operand_volumes':volumes}

def main():
 resources();acquire(slots=2)
 try:
  resources();assert not O.exists();data=json.loads((I/'geometry.json').read_text());branch=json.loads((BR/'branch.json').read_text())
  for folder in [I,BR]:assert json.loads((folder/'cpu-firsthits.json').read_text())['status']=='PASS_CPU_PROPOSED_BARK_SEEDS'
  bpy.ops.wm.read_factory_settings(use_empty=True);parts={p['source_obstacle_context']:p for p in data['parts']};data['parts']=[union(parts[3],branch,[2,3]),union(parts[4],parts[5],[4,5])];data['complete_wood_union']={'inputs':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in [I/'geometry.json',BR/'branch.json']},'limits':['Private four-stem geometry; inferred lower connections require native and oblique self-review.','Canopy and exact known-source material ownership remain unfinished.','Source metadata and gameplay are unchanged.']};O.mkdir();(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print([{k:v for k,v in p.items()if k not in ['vertices','faces']}for p in data['parts']])
 finally:release()
if __name__=='__main__':main()
