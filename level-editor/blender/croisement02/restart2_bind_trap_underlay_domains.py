"""Bind proposed trap underlay rays to unchanged approved receivers."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release
BASE=OUT/'restart2-state/trap-underlay-domain-audit-v1';DEST=OUT/'restart2-state/trap-underlay-receivers-v1'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 ledger=json.loads((OUT/'restart2-state/receiver-rebind-v2/report.json').read_text());acquire()
 try:
  DEST.mkdir();vertices=[];faces=[];owners=[];objects=[]
  for model in ledger['models']:
   path=Path(model['path']);assert sha(path)==model['sha256'];bpy.ops.wm.open_mainfile(filepath=str(path))
   if model['receiver']=='bank':bpy.context.window.scene=bpy.data.scenes[json.loads((path.parent/'workspace.json').read_text())['scene_name']]
   bpy.context.view_layer.update()
   for row in ledger['objects']:
    if row['receiver']!=model['receiver']:continue
    obj=bpy.data.objects[row['name']];assert np.max(abs(np.array(obj.matrix_world)-np.array(row['matrix_world'])))<1e-6;obj.data.calc_loop_triangles();offset=len(vertices);vertices.extend([obj.matrix_world@v.co for v in obj.data.vertices]);faces.extend([tuple(offset+i for i in t.vertices)for t in obj.data.loop_triangles]);owners.extend([len(objects)]*len(obj.data.loop_triangles));objects.append(row)
  tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True);domain=np.array(Image.open(BASE/'gray-candidate.png'))>0;labels=np.full(domain.shape,-1,np.int16);hits=[]
  for y,x in np.argwhere(domain):
   hit,normal,face,distance=tree.ray_cast(Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000,-RAY)
   if hit is None:raise ValueError('Unbound underlay ray')
   owner=owners[face];labels[y,x]=owner;hits.append([int(x),int(y),owner,*hit,face])
  records=[]
  for i,row in enumerate(objects):
   mask=labels==i;file=DEST/f'receiver-{i:02}.png';Image.fromarray(mask.astype('uint8')*255).save(file);records.append({'index':i,**row,'pixels':int(mask.sum()),'mask_sha256':sha(file)})
  np.savez_compressed(DEST/'bindings.npz',hits=np.array(hits));write_json(DEST/'report.json',{'status':'Bound current physical receiver domain; material fill packet still requires review','source_domain_sha256':sha(BASE/'gray-candidate.png'),'models':ledger['models'],'receivers':records,'total_pixels':len(hits),'limits':['Only receiver0 is confirmed gray by the approved ground atlas.','Bank candidates require separate material inspection; preserve already approved inferred brown material.','No foreign scenery or native dynamic image files are changed.']});print([(r['name'],r['pixels'])for r in records])
 finally:release()
if __name__=='__main__':main()
