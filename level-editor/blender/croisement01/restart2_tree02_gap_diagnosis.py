"""Read-only exact atlas-texel diagnostics for remaining inferred texture gaps."""
import collections,hashlib,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from texture_camera import orthographic_extents
from generated_visibility import far_plane,bounded_origin,visible_sample
R=ROOT/'level-editor/work/croisement01-refinement/restart2';case=R/'approved-tree02-fill-v1/croisement01-tree-02';b=case/'baked-v2-two-sided-crown';e=case/'experiment';out=R/'tree02-gap-diagnosis-v2';assert not (out/'report.json').exists();assert shutil.disk_usage(R).free>=10*1024**3+32*1024**2;assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
acquire();out.mkdir(exist_ok=True);model=b/'worker.blend';digest=hashlib.sha256(model.read_bytes()).hexdigest();bpy.ops.wm.open_mainfile(filepath=str(model));manifest=json.loads((e/'views.json').read_text());validation=json.loads((b/'validation.json').read_text());mask=np.asarray(Image.open(e/'mask.png').convert('RGBA'))[::-1];sheet_h,sheet_w=mask.shape[:2];vertices=[];triangles=[];owners=[];objects={name:bpy.data.objects[name] for name in manifest['object_names']}
for name,o in objects.items():
 base=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);o.data.calc_loop_triangles();triangles.extend(tuple(base+i for i in t.vertices) for t in o.data.loop_triangles);owners.extend((name,t.polygon_index) for t in o.data.loop_triangles)
tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);cameras=[(v,Matrix(v['camera_matrix_world']).inverted(),Matrix(v['camera_matrix_world']).to_3x3()@Vector((0,0,1))) for v in manifest['views']];records={};finite_planes={v['index']:far_plane(vertices,direction) for v,_,direction in cameras}
for entry in validation['layers'][0]['objects']:
 o=objects[entry['object']];mesh=o.data;a=np.load(entry['texel_provenance']['path'])['ownership'];h,w=a.shape;used={f.material_index for f in mesh.polygons};uvnames={n.uv_map for slot,mat in enumerate(mesh.materials) if slot in used and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='UVMAP' and n.uv_map};assert len(uvnames)==1;uv=mesh.uv_layers[next(iter(uvnames))].data;byface=collections.defaultdict(list)
 for tri in mesh.loop_triangles:byface[tri.polygon_index].append(tri)
 counts=collections.Counter();details=[]
 for f in mesh.polygons:
  uvs=np.array([uv[i].uv[:] for i in f.loop_indices]);center=uvs.mean(axis=0);x=max(0,min(w-1,int(center[0]*w)));y=max(0,min(h-1,int(center[1]*h)))
  if a[y,x]:continue
  picked=None
  for tri in byface[f.index]:
   triangle_uv=np.array([uv[i].uv[:] for i in tri.loops]);matrix=np.vstack([triangle_uv.T,np.ones(3)])
   if abs(np.linalg.det(matrix))<1e-18:continue
   weights=np.linalg.solve(matrix,np.array([(x+.5)/w,(y+.5)/h,1.]))
   if min(weights)>=-1e-6:picked=(tri,weights);break
  if picked is None:counts['center_texel_outside_triangle']+=1;continue
  tri,weights=picked;point=sum(((o.matrix_world@mesh.vertices[i].co)*float(z) for i,z in zip(tri.vertices,weights)),Vector((0,0,0)));normal=(o.matrix_world.to_3x3().inverted().transposed()@f.normal).normalized();tri_normal=(o.matrix_world.to_3x3().inverted().transposed()@tri.normal).normalized();row={'face':f.index,'atlas_texel':[x,y],'weights':weights.tolist(),'views':[]};visible=False;reasons=collections.Counter()
  if min(weights)<-1e-6:counts['center_texel_outside_triangle']+=1;continue
  for v,inverse,direction in cameras:
   local=inverse@point;horizontal,vertical=orthographic_extents(v);crop=v['crop'];px=crop['left']+(.5+local.x/horizontal)*crop['width'];py=sheet_h-crop['top']-(.5-local.y/vertical)*crop['height'];ix,iy=int(np.floor(px)),int(np.floor(py));dot=normal.dot(direction);score=abs(dot) if o.name=='Tree02 inferred crown' else dot;near=tree.ray_cast(point+direction*2000,-direction,4000);far=tree.ray_cast(point+direction*100000,-direction);bound=tree.ray_cast(Vector(bounded_origin(point,direction,finite_planes[v['index']])),-direction);be=(bound[0]-point).length if bound[0] is not None else None;bounded_same=bound[2] is not None and owners[bound[2]]==(o.name,f.index);same=near[2] is not None and owners[near[2]]==(o.name,f.index);visible|=same;ne=(near[0]-point).length if near[0] is not None else None;fe=(far[0]-point).length if far[0] is not None else None
   inside=crop['left']<=ix<crop['left']+crop['width'] and sheet_h-crop['top']-crop['height']<=iy<sheet_h-crop['top'];protected=bool(mask[iy,ix,3]>=128) if inside else None
   reason='facing' if score<=.12 else 'outside_tile' if not inside else 'protected_review_pixel' if protected else 'far_visibility' if fe is None or fe>.02 else 'eligible';reasons[reason]+=1
   if same or reason=='eligible':row['views'].append(dict(view=v['index'],facing=dot,triangle_facing=tri_normal.dot(direction),score=score,bounded_error=be,bounded_same_owner=bounded_same,mask_protected=protected,sheet_pixel=[px,py],reason=reason,near_error=ne,far_error=fe,near_owner=None if near[2] is None else owners[near[2]],far_owner=None if far[2] is None else owners[far[2]]))
  if not visible:counts['not_visible_at_exact_atlas_sample']+=1;continue
  classification='eligible_but_unfilled' if reasons['eligible'] else 'no_facing_camera' if reasons['facing']==8 else 'far_precision_or_occlusion' if any(v['reason']=='far_visibility' and v['near_error'] is not None and v['near_error']<.02 for v in row['views']) else 'review_mask_or_other_occlusion';counts[classification]+=1;row['classification']=classification;row['all_camera_reasons']=dict(reasons)
  if o.name=='Tree02 inferred crown' or sum(r['classification']==classification for r in details)<40:details.append(row)
 records[o.name]={'counts':dict(counts),'examples':details};print(o.name,dict(counts),flush=True)
assert hashlib.sha256(model.read_bytes()).hexdigest()==digest
(out/'report.json').write_text(json.dumps(dict(model_sha256=digest,objects=records,scope='Exact center texel inside each triangle, existing one-sided wood and two-sided crown eligibility. Same opaque geometry BVH; face-center census was approximate. No edits.'),indent=2)+'\n');assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file())<=32*1024**2
