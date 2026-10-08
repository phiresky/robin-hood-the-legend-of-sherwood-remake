"""Read saved wood shading around scoped joins without saving or rendering."""
import argparse,hashlib,json,sys
from pathlib import Path
import numpy as np
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement02-refinement'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def stats(values):
 a=np.asarray(values,float)
 return dict(count=len(a),maximum=float(a.max()) if len(a) else None,p95=float(np.percentile(a,95)) if len(a) else None,median=float(np.median(a)) if len(a) else None)
def angle(a,b):return float(np.degrees(np.arccos(np.clip(np.dot(a,b)/(np.linalg.norm(a)*np.linalg.norm(b)),-1,1))))
def inspect(path,tree,cut):
 before=sha(path);bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
 records=[];upper={}
 for obj in bpy.data.collections['Croisement02 Working'].all_objects:
  if obj.type!='MESH' or obj.get('asset_group')!=f'croisement02-tree-{tree}' or obj.get('projection_component')=='crown':continue
  mesh=obj.data;world=np.array([obj.matrix_world@v.co for v in mesh.vertices]);matrix=np.array(obj.matrix_world.to_3x3().inverted().transposed());loops={};auto=[];bandfaces=[]
  for face in mesh.polygons:
   points=world[list(face.vertices)];center=points[:,2].mean();band=abs(center-cut)<=2
   if band:bandfaces.append(face)
   for loop_id in face.loop_indices:
    vertex=mesh.loops[loop_id].vertex_index;n=matrix@np.array(mesh.corner_normals[loop_id].vector);n/=np.linalg.norm(n)
    if band:
     loops.setdefault(vertex,[]).append((center>=cut,n));auto.append(angle(n,matrix@np.array(mesh.vertices[vertex].normal)))
    if points[:,2].min()>=cut+.01:
     key=tuple(np.round(world[vertex],5));upper.setdefault(key,[]).append(n)
  split=[];cross=[]
  for values in loops.values():
   split.extend(angle(a[1],b[1]) for i,a in enumerate(values) for b in values[i+1:])
   hi=[n for side,n in values if side];lo=[n for side,n in values if not side]
   cross.extend(angle(a,b) for a in hi for b in lo)
  records.append(dict(object=obj.name,source_node=obj.get('source_node'),has_custom_normals=mesh.has_custom_normals,total_faces=len(mesh.polygons),total_flat_faces=sum(not f.use_smooth for f in mesh.polygons),band_faces=len(bandfaces),band_flat_faces=sum(not f.use_smooth for f in bandfaces),band_sharp_edges=int(sum(e.use_edge_sharp and np.max(np.abs(world[list(e.vertices),2]-cut))<=2 for e in mesh.edges)),band_corner_vs_vertex=stats(auto),band_same_vertex_corner_spread=stats(split),interface_upper_lower_corner_angles=stats(cross)))
 if sha(path)!=before:raise ValueError('Read-only audit mutated input')
 averages={k:np.mean(v,axis=0) for k,v in upper.items()}
 return dict(path=str(path),sha256=before,objects=records),averages

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
 if a.output.exists():raise FileExistsError(a.output)
 specs={32:('root-stem-round-3/assets/croisement02-tree-32/model.blend','tree32-root-research/continuous-field-smooth-source-v3/model.blend',125),38:('root-stem-round-2/assets/croisement02-tree-38/model.blend','tree38-root-research/continuous-field-smooth-source-v4/model.blend',115)}
 # Pin all exact inputs before acquiring the lane or opening the first file.
 pins={str(OUT/name):sha(OUT/name) for old,new,_ in specs.values() for name in [old,new]};records=[]
 acquire()
 try:
  for tree,(old,new,cut) in specs.items():
   baseline,bn=inspect(OUT/old,tree,cut);candidate,cn=inspect(OUT/new,tree,cut);common=bn.keys()&cn.keys();drift=[angle(bn[k],cn[k]) for k in common]
   records.append(dict(tree=tree,cut_z=cut,baseline=baseline,candidate=candidate,retained_upper_matched_vertices=len(common),retained_upper_unmatched_baseline=len(bn.keys()-cn.keys()),retained_upper_corner_normal_change=stats(drift)))
  for path,digest in pins.items():
   if sha(Path(path))!=digest:raise ValueError('Pinned input changed')
  report=dict(status='Read-only saved normal audit; no geometry or appearance approval',records=records,input_sha256=pins,limitations=['Normals are compared at shared retained world vertices above the cut; polygon subdivision can alter weighting.','Interface bands span two world units either side. No material/light render is performed.'])
  data=json.dumps(report,indent=2)+'\n'
  if len(data.encode())>2*1024*1024:raise ValueError('Output budget exceeded')
  a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(data);print(data)
 finally:release()
if __name__=='__main__':main()
