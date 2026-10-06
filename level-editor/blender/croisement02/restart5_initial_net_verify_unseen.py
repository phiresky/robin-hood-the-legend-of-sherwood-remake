"""Reconstruct bounded texture continuation and verify saved PNG quantization."""
import json
from pathlib import Path
import bpy,numpy as np
from evidence_io import sha,write_json
from bake_texture_candidate import pixels
from project_reviewed_texture import _read

def atlas(scene,name):
 obj=scene.objects[name];mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'));node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);return pixels(node.image).copy()

def verify(w):
 proof=json.loads((w/'continuation.json').read_text());scene=bpy.context.scene;actual={r['object']:atlas(scene,r['object'])for r in proof['objects']};parent=w.parent/'native-retained-v2/worker.blend';assert sha(parent)==proof['parent_model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(parent));rows=[]
 for r in proof['objects']:
  name=r['object'];a=atlas(bpy.context.scene,name);b=a.copy();mapping=np.load(w/(name.replace(' ','-')+'.npz'));tx,ty=mapping['target_xy'].T;dx,dy=mapping['donor_xy'].T;b[ty,tx,:3]=a[dy,dx,:3]
  if r.get('projected_raw_generated_donors'):
   raw=_read(r['raw_generated_image']);m=np.load(w/'ground-generated-native-projection.npz')
   for index,row in zip(m['target_indices'],m['sheet_xy_distance']):
    px,py,_=row;x0=int(np.floor(px-.5));y0=int(np.floor(py-.5));ax=px-.5-x0;ay=py-.5-y0;color=(raw[y0,x0,:3]*(1-ax)+raw[y0,x0+1,:3]*ax)*(1-ay)+(raw[y0+1,x0,:3]*(1-ax)+raw[y0+1,x0+1,:3]*ax)*ay;b[ty[index],tx[index],:3]=color
  target=np.zeros(a.shape[:2],bool);target[ty,tx]=True;got=actual[name];assert np.array_equal(got[~target],a[~target]);assert np.array_equal(got[:,:,3],a[:,:,3]);error=float(np.max(np.abs(got-b)));assert error<=1/255+1e-6,(name,error);rows.append(dict(object=name,noneditable_and_padding_RGBA_exact=True,alpha_exact=True,maximum_saved_float_error=error,maximum_saved_RGBA8_error=int(np.max(np.abs(np.rint(got*255)-np.rint(b*255))))))
 bpy.ops.wm.open_mainfile(filepath=str(w/'worker.blend'));write_json(w/'reopened-generated-guard.json',dict(model_sha256=proof['model_sha256'],status='PASS',objects=rows,scope='Same-object donor reconstruction exact before PNG save; newly interpolated colors differ only by bounded 8-bit PNG quantization. Noneditable source, padding and alpha byte-exact.'));return rows
