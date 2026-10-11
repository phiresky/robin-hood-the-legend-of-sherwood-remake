"""Distinguish genuine unfilled surface texels from unused atlas padding."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];O=ROOT/'level-editor/work/york-refinement/restart2'/(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'restart60-winch-chain-texture-bake-v1')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
import bpy,numpy as np
bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));report=json.loads((O/'validation.json').read_text());rows=[]
for item in report['layers'][0]['objects']:
 obj=bpy.context.scene.objects[item['object']];obj.data.calc_loop_triangles();owner=np.load(item['texel_provenance']['path'])['ownership'];h,w=owner.shape;domain=np.zeros((h,w),bool)
 for tri in obj.data.loop_triangles:
  mat=obj.data.materials[obj.data.polygons[tri.polygon_index].material_index];uvnode=next(n for n in mat.node_tree.nodes if n.type=='UVMAP');uv=obj.data.uv_layers[uvnode.uv_map];pts=np.array([list(uv.data[i].uv) for i in tri.loops])*[w,h]
  lo=np.maximum(0,np.floor(pts.min(axis=0)).astype(int));hi=np.minimum([w,h],np.ceil(pts.max(axis=0)).astype(int))
  if np.any(hi<=lo):continue
  yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];p=np.stack([xx+.5,yy+.5],-1);a,b,c=pts;v0=b-a;v1=c-a;den=v0[0]*v1[1]-v1[0]*v0[1]
  if abs(den)<1e-12:continue
  q=p-a;s=(q[:,:,0]*v1[1]-v1[0]*q[:,:,1])/den;t=(v0[0]*q[:,:,1]-q[:,:,0]*v0[1])/den;domain[lo[1]:hi[1],lo[0]:hi[0]]|=(s>=-1e-6)&(t>=-1e-6)&(s+t<=1+1e-6)
 rows.append({'object':obj.name,'surface_texels':int(domain.sum()),'unfilled_surface_texels':int(((owner==0)&domain).sum()),'source_texels':int(((owner==1)&domain).sum()),'generated_texels':int(((owner==2)&domain).sum()),'edge_repaired_texels':int(((owner==3)&domain).sum())})
out={'model_sha256':hashlib.sha256((O/'worker.blend').read_bytes()).hexdigest(),'method':'Pixel-center rasterization of saved UV triangles against bake provenance; zero flags outside triangles are padding, not missing surface.','totals':{k:sum(r[k] for r in rows) for k in ['surface_texels','unfilled_surface_texels','source_texels','generated_texels','edge_repaired_texels']},'objects':rows};(O/'surface-coverage.json').write_text(json.dumps(out,indent=2)+'\n');print(out['totals'])
