"""Read-only face-center provenance and visible-unfilled census of the saved fill."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';b=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v1-luminance';e=b.parent/'experiment';out=b/'face-provenance-census.json';assert not out.exists();acquire();before=sha(b/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(b/'worker.blend'));frames=json.loads((e/'views.json').read_text());report=json.loads((b/'validation.json').read_text());vertices=[];faces=[];offsets={};objects={name:bpy.data.objects[name] for name in frames['object_names']}
for name,o in objects.items():
 offsets[name]=len(faces);offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
bvh=BVHTree.FromPolygons(vertices,faces);counts={}
for entry in report['layers'][0]['objects']:
 o=objects[entry['object']];mesh=o.data;a=np.load(entry['texel_provenance']['path'])['ownership'];height,width=a.shape;used={f.material_index for f in mesh.polygons};uv_names={n.uv_map for slot,mat in enumerate(mesh.materials) if slot in used and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='UVMAP' and n.uv_map};assert len(uv_names)==1;uv=mesh.uv_layers[next(iter(uv_names))].data;row={'counts':{},'visible_unfilled_face_centers':0,'visible_unfilled_area':0.,'unfilled_area':0.,'examples':[]}
 for face in mesh.polygons:
  p=sum((uv[i].uv for i in face.loop_indices),start=Vector((0,0)))/len(face.loop_indices);x=max(0,min(width-1,int(p.x*width)));y=max(0,min(height-1,int(p.y*height)));value=int(a[y,x]);row['counts'][value]=row['counts'].get(value,0)+1
  if value:continue
  row['unfilled_area']+=face.area;center=o.matrix_world@face.center
  for view in frames['views']:
   camera=Matrix(view['camera_matrix_world']);local=camera.inverted()@center;scale=view['ortho_scale']
   if max(abs(local.x),abs(local.y))>scale/2:continue
   direction=camera.to_3x3()@Vector((0,0,1));hit=bvh.ray_cast(center+direction*2000,-direction,4000)
   if hit[2]==offsets[o.name]+face.index:
    row['visible_unfilled_face_centers']+=1;row['visible_unfilled_area']+=face.area
    if len(row['examples'])<12:row['examples'].append(dict(face=face.index,view=view['index'],area=face.area))
    break
 counts[o.name]=row
assert sha(b/'worker.blend')==before
out.write_text(json.dumps(dict(model_sha256=before,objects=counts,limitation='Face-center sample census only, eight fixed orthographic cameras and target geometry BVH. Does not claim all texels or transparency-aware native ownership.'),indent=2)+'\n');print(json.dumps(counts),flush=True)
