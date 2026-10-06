"""Inventory saved source-derived cluster materials without modifying the model."""
import hashlib,json,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment/cluster-geometry-v6';out=p/'appearance-audit.json';assert not out.exists();model=p/'worker.blend';assert sha(model)=='9e8ece3cc693a2ef387333c762e8a85d2cc3fa808a7726c821bebd9233cb0272';acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(model));o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');rows=[]
  for slot in sorted({f.material_index for f in o.data.polygons}):
   m=o.data.materials[slot];images=[]
   for n in m.node_tree.nodes:
    if n.type!='TEX_IMAGE' or not n.image:continue
    im=n.image;a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);a=a.reshape(im.size[1],im.size[0],4);solid=a[:,:,3]>.5;rgb=a[:,:,:3];neutral=(rgb.max(2)-rgb.min(2)<.015)&(rgb.mean(2)>.15)&(rgb.mean(2)<.85)&solid
    images.append({'name':im.name,'size':list(im.size),'float_rgba_sha256':hashlib.sha256(a.tobytes()).hexdigest(),'opaque_pixels':int(solid.sum()),'near_neutral_opaque_pixels':int(neutral.sum()),'packed':bool(im.packed_file)})
   rows.append({'slot':slot,'name':m.name,'faces':sum(f.material_index==slot for f in o.data.polygons),'images':images})
  out.write_text(json.dumps({'status':'READ ONLY saved appearance inventory','model_sha256':sha(model),'material_rows':rows,'limits':['Near-neutral color is descriptive, not proof of missing texture; source bark or leaf highlights may be neutral.','Existing source-derived inferred foliage and prior API wood/offmap material remain separate appearance scope; no new synthesis performed.']},indent=2)+'\n');print(out)
 finally:release()
if __name__=='__main__':main()
