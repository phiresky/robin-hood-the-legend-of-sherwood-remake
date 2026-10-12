"""Verify partitioned derivative positions, independent UVs and original RGB bytes."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from scipy.spatial import cKDTree
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
import lossy_assets as la
O=R/'level-editor/work/active/croisement02';B=O/'approved-round-wood-integration-oct11-v1';read=lambda p:json.loads(p.read_text());h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main(n):
 D=B/f'tree-{n}-v1';proof=read(D/'export.json');model=Path(proof['source_model']);assert h(model)==proof['source_sha256'];assert h(D/'exact/model.glb')==proof['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();doc,buffers,_=la.read_glb(D/'exact/model.glb');pivot=np.array(proof['pivot']);converted={r['object']:r for r in proof['conversion_records']};rows=[]
 for node in doc['nodes']:
  if'mesh'not in node or 'crown'in node.get('name','').lower():continue
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
    uv=la.accessor_array(doc,buffers,index).copy();uv[:,1]=1-uv[:,1];query=np.column_stack((pos,uv*100));scores={name:float(t.query(query,distance_upper_bound=.002)[0].max())for name,t in joint.items()};best=min(scores,key=scores.get);assert scores[best]<.002,(node['name'],attr,scores);matches[int(attr.split('_')[1])]=best;attributes.append(dict(attribute=attr,source_uv=best,maximum_joint_error=scores[best]))
   mat=doc['materials'][primitive['material']];leaf=mat.get('extras',{}).get('exact_binary_partition_leaf')
   if leaf is not None:
    original=converted[ob.name]['leaves'][str(leaf)]
    if original[0]=='image':
     assert matches[mat['emissiveTexture'].get('texCoord',0)]==original[2],(mat['name'],matches,original)
     image_index=doc['textures'][mat['emissiveTexture']['index']]['source'];image=doc['images'][image_index]
     if 'uri'in image:payload=(D/'exact'/image['uri']).read_bytes()
     else:
      view=doc['bufferViews'][image['bufferView']];payload=buffers[view.get('buffer',0)][view.get('byteOffset',0):view.get('byteOffset',0)+view['byteLength']]
     assert hashlib.sha256(payload).hexdigest()==hashlib.sha256(bytes(bpy.data.images[original[1]].packed_file.data)).hexdigest(),(mat['name'],original[1],'wrong RGB branch image')
  assert count==expected_triangles,(ob.name,count,expected_triangles);assert maximum<.001;rows.append(dict(object=ob.name,source_triangles=len(ob.data.loop_triangles),exact_partition_triangles=expected_triangles,exported_triangles=count,maximum_position_error=maximum,uvs=attributes))
 result=dict(status='PASS_APPROVED_WOOD_SURFACE_UV_IMAGE_GUARD',source_model_sha256=proof['source_sha256'],export_sha256=proof['model_sha256'],source_model_unchanged=h(model)==proof['source_sha256'],receivers=rows,scope='Raw exported wood only; crown is discarded and retained from installed model via separate exact merge proof.')
 (D/'source-surface-guard.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'],flush=True)
if __name__=='__main__':
 acquire(slots=2)
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
