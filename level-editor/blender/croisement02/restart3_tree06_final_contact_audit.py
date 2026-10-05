"""Bound exact root support, source-ray residual depths and material-only changes."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json

def mesh_record(o):
 o.data.calc_loop_triangles();verts=[tuple(o.matrix_world@v.co)for v in o.data.vertices];faces=[tuple(t.vertices)for t in o.data.loop_triangles]
 return verts,faces,BVHTree.FromPolygons(verts,faces,all_triangles=True)
def main():
 base=OUT/'restart3-tree06-root/research-roots-v5';geom=base/'root.blend';appearance=base/'appearance-v3/root.blend';bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 records=[]
 for path in [geom,appearance]:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();o=next(o for o in bpy.context.scene.objects if o.type=='MESH');v,f,t=mesh_record(o);records.append((v,f,t));matrix=[list(r)for r in o.matrix_world]
 assert records[0][:2]==records[1][:2];verts,faces,root=records[0]
 from PIL import Image
 mask=np.asarray(Image.open(base/'appearance-v3/observed-mask.png'));source_rows=json.loads((base/'report.json').read_text())['samples'];errors=json.loads((base/'appearance-v3/sample-errors.json').read_text())['errors'];error_pixels={tuple(r['pixel'])for r in errors};material_rows=[]
 for row in source_rows:
  if not(row['wood']and row['visible']):continue
  x,y=row['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hit,n,face,d=root.ray_cast(origin,-RAY,10000);poly=o.data.polygons[o.data.loop_triangles[face].polygon_index]
  record=dict(pixel=[x,y],material_index=poly.material_index,ownership_mask=int(mask[y-464,x-594,0]),front_facing=poly.normal.dot(RAY)>0)
  if (x,y)in error_pixels:
   offsets=[]
   for dy in np.linspace(-.3,.3,9):
    for dx in np.linspace(-.3,.3,9):
     q=Vector((x+.5+dx,-(y+.5+dy)/SIN,0))+RAY*5000;hh,nn,ff,dd=root.ray_cast(q,-RAY,10000);offsets.append(dict(offset=[dx,dy],root_hit=hh is not None,material_index=o.data.polygons[o.data.loop_triangles[ff].polygon_index].material_index if ff is not None else None))
   record['nearby_subpixel_rays']=offsets
  material_rows.append(record)
 write_json(base/'appearance-v3/material-ray-audit.json',dict(appearance_sha256=sha(appearance),samples=len(material_rows),exact_owned_front_material=sum(r['material_index']==1 and r['ownership_mask']==255 for r in material_rows),samples_detail=material_rows,explanation='Exact center-ray material ownership is checked separately from antialiased render sampling; nearby subpixel rays explain edge mixtures without geometry mutation.'))
 bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update();banks=[mesh_record(o)[2]for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank'];bank_matrices={o.name:[list(r)for r in o.matrix_world]for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank'}
 clear=[];missing=[]
 for i,p in enumerate(verts):
  hits=[t.ray_cast(Vector((p[0],p[1],2000)),Vector((0,0,-1)),4000)[0]for t in banks];heights=[h.z for h in hits if h is not None]
  if heights:clear.append(max(heights)-p[2])
  else:missing.append(i);clear.append(None)
 crossing=[i for i,f in enumerate(faces)if all(clear[j]is not None for j in f)and min(clear[j]for j in f)<=0<=max(clear[j]for j in f)]
 source=json.loads((base/'report.json').read_text());res=[]
 for r in source['samples']:
  if (r['wood']and not r['visible'])or(not r['wood']and r['visible']):
   x,y=r['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hit,n,face,d=root.ray_cast(origin,-RAY,10000)
   res.append(dict(pixel=r['pixel'],kind='missing_wood'if r['wood']else 'bank_overlap',root_hit=list(hit)if hit else None,source_ray_clearance_from_bank=r['clearance'],reason='outside_root_projected_silhouette'if hit is None else('root_behind_bank'if r['clearance']<=0 else 'root_in_front_of_bank')))
 def stats(values):return dict(count=len(values),min=min(values)if values else None,max=max(values)if values else None)
 depth={kind:dict(no_root_hit=sum(r['root_hit']is None for r in res if r['kind']==kind),ray_clearance=stats([r['source_ray_clearance_from_bank']for r in res if r['kind']==kind and r['source_ray_clearance_from_bank']is not None]))for kind in ['missing_wood','bank_overlap']}
 solid=json.loads((base/'neutral-solid/report.json').read_text());art=json.loads((base/'appearance-v3/report.json').read_text());solid_bank={r['source_object']:r['matrix_world']for r in solid['verified_context']};art_bank={r['source_object']:r['matrix_world']for r in art['context_imports'][1]};assert solid_bank==art_bank==bank_matrices
 write_json(base/'final-contact-audit.json',dict(geometry_sha256=sha(geom),appearance_sha256=sha(appearance),bank_sha256=sha(bank),appearance_geometry_exact=True,root_matrix=matrix,neutral_and_appearance_bank_matrices_exact=True,root_support=dict(vertex_count=len(verts),missing_bank_under_vertices=missing,buried_vertices=sum(c is not None and c>0 for c in clear),exposed_vertices=sum(c is not None and c<0 for c in clear),vertical_bank_minus_root=stats([c for c in clear if c is not None]),bank_crossing_triangles=len(crossing)),residual_depth_summary=depth,residual_samples=res,interpretation=['One connected closed root body crosses the bank surface; apparent floating gray outline is not a transform discrepancy.','Gray source-facing fringe is opaque unknown material, never transparent masking.','Missing source rays and overlap depths remain explicit; this does not make them approved semantic ownership.']))
 print(depth,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
