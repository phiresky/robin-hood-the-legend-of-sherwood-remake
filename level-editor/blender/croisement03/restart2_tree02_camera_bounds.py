"""Audit complete saved-geometry review frusta and invalidate clipped diagnostics."""
import sys,json,math
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main():
 out=B/'tree02-isolated-prototype-v2';review=out/'review-unclipped-v1';assert (review/'view-receipt.json').exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));s=bpy.data.scenes['Tree02 isolated'];bpy.context.window.scene=s;s.view_layers.update();points=[o.matrix_world@v.co for o in s.objects if o.type=='MESH' for v in o.data.vertices];rows=[]
  rendered=json.loads((review/'actual/renders.json').read_text())['renders']
  for i in range(8):
   c=s.objects[f'Tree02 view{i}'];inv=c.matrix_world.inverted();local=[inv@p for p in points];depth=[-p.z for p in local];mn,mx=min(depth),max(depth);derived_near=max(.1,mn-25);derived_far=mx+25;assert .1<=derived_near and 10000>=derived_far;half=c.data.ortho_scale/2;xy=[min(p.x for p in local),min(p.y for p in local),max(p.x for p in local),max(p.y for p in local)];assert all(abs(v)<half for v in xy),(i,xy,half)
   matrices=[r['camera_matrix'] for r in rendered if r['camera']==c.name];assert matrices and all(m==[list(row) for row in c.matrix_world] for m in matrices)
   rows.append(dict(view=i,geometry_depth=[mn,mx],derived_depth_with_margin=[derived_near,derived_far],actual_review_clip=[.1,10000],source_camera_clip=[c.data.clip_start,c.data.clip_end],source_far_clipped_vertices=sum(d>c.data.clip_end for d in depth),xy_bounds=xy,ortho_half_extent=half,camera_projection_matrix_exact=True))
  write_json(review/'complete-frustum-guard.json',dict(status='PASS all saved geometry inside every corrected frustum with25-unit depth margin',model_sha256=sha(out/'worker.blend'),view_receipt_sha256=sha(review/'view-receipt.json'),vertices=len(points),cameras=rows,invalidated=['The initial v2 actual/solid/alpha-gray sheets used far1000 at camera distance1000 and clipped geometry.','Stem thinness, missing lower trunk and disconnected branch claims based on those sheets are invalid.','The upper cloud/lower strip concern must be assessed against the corrected complete images, not assumed resolved.'],limits=['Corrected review uses broad depth bounds that contain the complete vertex-derived range plus margin. Original game camera direction/orthographic projection unchanged.']))
  print('PASS complete camera bounds',len(points),[r['source_far_clipped_vertices'] for r in rows])
 finally:release()
if __name__=='__main__':main()
