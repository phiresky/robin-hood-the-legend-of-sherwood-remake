"""Guard source surfaces and exported UV coordinates against the approved Tree07 worker."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from scipy.spatial import cKDTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
import lossy_assets as la
from render_slots import acquire,release
D=ROOT/'level-editor/work/croisement02-refinement/restart7-tree07-approved-export-v1'
def main():
 proof=json.loads((D/'export.json').read_text());bpy.ops.wm.open_mainfile(filepath=proof['source_model']);bpy.context.view_layer.update();doc,buffers,_=la.read_glb(D/'exact/model.glb');pivot=np.array(proof['pivot']);rows=[]
 for node in doc['nodes']:
  if 'mesh'not in node:continue
  obj=bpy.data.objects[node['name']];obj.data.calc_loop_triangles();world=np.array([tuple(obj.matrix_world@v.co)for v in obj.data.vertices]);tree=cKDTree(world);triangles=len(obj.data.loop_triangles);exportedtri=0;maxdist=0.;uvrows=[];loops=np.array([l.vertex_index for l in obj.data.loops]);source_uv={uv.name:np.array([tuple(x.uv)for x in uv.data])for uv in obj.data.uv_layers}
  joint={name:cKDTree(np.column_stack([world[loops],xy*100]))for name,xy in source_uv.items()}
  for primitive in doc['meshes'][node['mesh']]['primitives']:
   pos=la.accessor_array(doc,buffers,primitive['attributes']['POSITION'])+pivot;dist,_=tree.query(pos);maxdist=max(maxdist,float(dist.max()));exportedtri+=len(la.accessor_array(doc,buffers,primitive['indices']))//3
   for attr,index in primitive['attributes'].items():
    if not attr.startswith('TEXCOORD_'):continue
    uv=la.accessor_array(doc,buffers,index).copy();uv[:,1]=1-uv[:,1];query=np.column_stack([pos,uv*100]);scores={name:float(t.query(query)[0].max())for name,t in joint.items()};best=min(scores,key=scores.get);assert scores[best]<.002,(node['name'],attr,scores);uvrows.append(dict(attribute=attr,matched_source_uv=best,maximum_joint_position_uv_error=scores[best]))
  assert maxdist<.001,(obj.name,maxdist);assert triangles==exportedtri,(obj.name,triangles,exportedtri);rows.append(dict(object=obj.name,source_triangles=triangles,exported_triangles=exportedtri,maximum_source_position_error=maxdist,uv_layers=uvrows))
 imageproof=json.loads((D/'delivery-proof.json').read_text());source_image_hashes={i['packed_sha256']for o in proof['source_receivers']for i in o['images']};assert all(i['sha256']in source_image_hashes for i in imageproof['images']);result=dict(status='PASS source surfaces, per-vertex original UV correspondence, exact original image bytes',source_sha256=proof['source_sha256'],export_sha256=proof['model_sha256'],delivery_sha256=imageproof['delivery_glb_sha256'],source_receivers=rows,all_seven_exported_image_bytes_match_original_packed_images=True,native_rgb_and_alpha_bytes_exact=True,limitations=['Joint UV guard uses original UV layer at the same source vertex; appearance semantics additionally reviewed in actual WebGL.','Floating point world/pivot conversion is bounded below0.001worldunit; export triangle counts unchanged.']);(D/'source-surface-guard.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['object'],r['maximum_source_position_error'])for r in rows],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
