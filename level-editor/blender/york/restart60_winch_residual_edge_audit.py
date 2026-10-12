"""Inspect exact UV barycentrics for remaining visible residual atlas samples."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];O=ROOT/'level-editor/work/active/york/restart2/restart60-winch-chain-combined-v2'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
import bpy,numpy as np
def cross2(a,b):return a[0]*b[1]-a[1]*b[0]
bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));v=json.loads((O/'remaining-visibility.json').read_text());report=json.loads((O/'validation.json').read_text());rows=[]
for p in v['points']:
 if not p['witnesses']:continue
 o=bpy.context.scene.objects[p['object']];item=next(r for r in report['layers'][0]['objects'] if r['object']==o.name);owner=np.load(item['texel_provenance']['path'])['ownership'];h,w=owner.shape;o.data.calc_loop_triangles();matches=[]
 for t in o.data.loop_triangles:
  poly=o.data.polygons[t.polygon_index];mat=o.data.materials[poly.material_index];uv=o.data.uv_layers[next(n.uv_map for n in mat.node_tree.nodes if n.type=='UVMAP')];points=np.array([list(uv.data[i].uv) for i in t.loops])*[w,h];a,b,c=points;q=np.array(p['uv_pixel'][::-1])+.5-a;v0=b-a;v1=c-a;den=v0[0]*v1[1]-v1[0]*v0[1]
  if abs(den)<1e-10:continue
  s=(q[0]*v1[1]-v1[0]*q[1])/den;z=(v0[0]*q[1]-q[0]*v0[1])/den;weights=[1-s-z,s,z]
  if min(weights)<-1e-6:continue
  polyuv=np.array([list(uv.data[i].uv) for i in poly.loop_indices])*[w,h];dist=min(abs(cross2(polyuv[(i+1)%len(polyuv)]-x,np.array(p['uv_pixel'][::-1])+.5-x))/np.linalg.norm(polyuv[(i+1)%len(polyuv)]-x) for i,x in enumerate(polyuv));matches.append({'face':t.polygon_index,'triangle_barycentric':weights,'minimum_polygon_edge_distance_texels':float(dist),'uv_polygon':polyuv.tolist()})
 assert matches;rows.append({'object':o.name,'uv_pixel':p['uv_pixel'],'matches':matches})
result={'model_sha256':hashlib.sha256((O/'worker.blend').read_bytes()).hexdigest(),'points':rows};(O/'residual-edge-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(rows))
