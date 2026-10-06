"""Verify partitioned derivative positions, independent UVs and original RGB bytes."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from scipy.spatial import cKDTree
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
import lossy_assets as la
O=R/'level-editor/work/croisement02-refinement';B=O/'restart8-five-bark-approved-export-v1';read=lambda p:json.loads(p.read_text());h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main(n):
 D=B/f'tree-{n}-v1';proof=read(D/'export.json');model=Path(proof['source_model']);assert h(model)==proof['source_sha256'];assert h(D/'exact/model.glb')==proof['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();doc,buffers,_=la.read_glb(D/'exact/model.glb');pivot=np.array(proof['pivot']);converted={r['object']:r for r in proof['conversion_records']};rows=[]
 for node in doc['nodes']:
  if'mesh'not in node:continue
  ob=bpy.data.objects[node['name']];ob.data.calc_loop_triangles();world=np.array([tuple(ob.matrix_world@v.co)for v in ob.data.vertices]);uvs={u.name:np.array([x.uv[:]for x in u.data])for u in ob.data.uv_layers};expected_triangles=len(ob.data.loop_triangles)
  if ob.name in converted:
   record=converted[ob.name];path=Path(record['provenance']);assert h(path)==record['provenance_sha256'];data=np.load(path);ids=data['source_triangles'];weights=data['barycentric'];vi=data['source_vertex_indices'];loops=data['source_loop_indices'];expected_positions=np.einsum('tij,tjk->tik',weights,world[vi[ids]]).reshape(-1,3);expected_uv={name:np.einsum('tij,tjk->tik',weights,a[loops[ids]]).reshape(-1,2)for name,a in uvs.items()};expected_triangles=len(ids)
  else:
   loops=np.array([x.vertex_index for x in ob.data.loops]);expected_positions=world[loops];expected_uv=uvs
  tree=cKDTree(expected_positions);joint={name:cKDTree(np.column_stack((expected_positions,xy*100)))for name,xy in expected_uv.items()};count=0;maximum=0.;attributes=[]
  for primitive in doc['meshes'][node['mesh']]['primitives']:
   pos=la.accessor_array(doc,buffers,primitive['attributes']['POSITION'])+pivot;dist=tree.query(pos)[0];maximum=max(maximum,float(dist.max()));count+=len(la.accessor_array(doc,buffers,primitive['indices']))//3;matches={}
   for attr,index in primitive['attributes'].items():
    if not attr.startswith('TEXCOORD_'):continue
    uv=la.accessor_array(doc,buffers,index).copy();uv[:,1]=1-uv[:,1];query=np.column_stack((pos,uv*100));scores={name:float(t.query(query)[0].max())for name,t in joint.items()};best=min(scores,key=scores.get);assert scores[best]<.002,(node['name'],attr,scores);matches[int(attr.split('_')[1])]=best;attributes.append(dict(attribute=attr,source_uv=best,maximum_joint_error=scores[best]))
   mat=doc['materials'][primitive['material']];leaf=mat.get('extras',{}).get('exact_binary_partition_leaf')
   if leaf is not None:
    original=converted[ob.name]['leaves'][str(leaf)]
    if original[0]=='image':assert matches[mat['emissiveTexture'].get('texCoord',0)]==original[2],(mat['name'],matches,original)
  assert count==expected_triangles,(ob.name,count,expected_triangles);assert maximum<.001;rows.append(dict(object=ob.name,source_triangles=len(ob.data.loop_triangles),exact_partition_triangles=expected_triangles,exported_triangles=count,maximum_position_error=maximum,uvs=attributes))
 delivery=read(D/'delivery-proof.json');allowed={r['packed_sha256']for ob in proof['source_receivers']for r in ob['images']};assert all(im['sha256']in allowed for im in delivery['images']);result=dict(status='PASS exact surface partition/UV/image and delivery guards; actual WebGL review pending',source_model_sha256=proof['source_sha256'],export_sha256=proof['model_sha256'],delivery_sha256=delivery['delivery_glb_sha256'],all_exported_rgb_image_bytes_original=True,all_exported_positions_on_original_triangle=True,all_shader_uvs_match_correct_original_layer=True,source_model_unchanged=h(model)==proof['source_sha256'],receivers=rows,original_partition_guards=[dict(object=r['object'],maximum_area_fraction_error=r['maximum_area_fraction_error'],all_region_centroid_branches_exact=r['all_region_centroid_branches_exact'])for r in proof['conversion_records']],limitations=['Additional coplanar triangles encode exact binary material-region boundaries; physical surface unchanged.','Production sampler filtering and antialiasing require independent browser review.']);(D/'source-surface-guard.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'],flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
