"""Read-only texture provenance census on inferred off-map tree boughs."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';b=R/'approved-tree01-fill-v1/croisement01-tree-01/baked-v2-luminance';out=b/'bough-provenance-v2.json';assert not out.exists();acquire();before=sha(b/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(b/'worker.blend'));o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-029');report=json.loads((b/'layer-0.json').read_text());entry=next(x for x in report['objects'] if x['object']==o.name);a=np.load(entry['texel_provenance']['path'])['ownership'];height,width=a.shape;mesh=o.data;uv_names={n.uv_map for slot,mat in enumerate(mesh.materials) if slot in {f.material_index for f in mesh.polygons} and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='UVMAP' and n.uv_map};print('USED UV',uv_names,'LAYERS',[x.name for x in mesh.uv_layers]);assert len(uv_names)==1;uv=mesh.uv_layers[next(iter(uv_names))].data;counts={};s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
for face in mesh.polygons:
 center=o.matrix_world@face.center;region='off-map-upper-bough' if -center.y*s-center.z*c<0 else 'native-frame-wood';p=sum((uv[i].uv for i in face.loop_indices),start=__import__('mathutils').Vector((0,0)))/len(face.loop_indices);x=max(0,min(width-1,int(p.x*width)));y=max(0,min(height-1,int(p.y*height)));key=str(int(a[y,x]));row=counts.setdefault(region,{});row[key]=row.get(key,0)+1
assert sha(b/'worker.blend')==before;out.write_text(json.dumps(dict(model_sha256=before,provenance_sha256=sha(Path(entry['texel_provenance']['path'])),face_center_samples=counts,semantics=entry['texel_provenance']['semantics'],limitation='Face-center sample census only; silhouette and rendered visual review remain authoritative.'),indent=2)+'\n');print(out)
