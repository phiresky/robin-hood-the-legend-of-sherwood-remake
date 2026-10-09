"""Independently compare saved native UV loops and source image bytes after filling."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
BASE=OUT/'restart25-approved-state-materialization-v1'
def capture(model,names):
 bpy.ops.wm.open_mainfile(filepath=str(model));result={}
 for name in names:
  obj=bpy.data.objects[name];native='Initial native source projection' if name.startswith('Leaf-covered trap') else 'Native source projection';uv=np.asarray([v.uv[:] for v in obj.data.uv_layers[native].data],dtype=np.float32)
  materials=[]
  for mat in list(obj.data.materials)[:2]:
   materials.append([(n.name,hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest())for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image and n.image.packed_file])
  result[name]={'geometry':_geometry(obj),'native_uv':hashlib.sha256(uv.tobytes()).hexdigest(),'original_images':materials}
 return result

def main():
 cases=[]
 for key,rev in [('hole-initial','native-state-v1'),('hole-applied','native-state-v1'),('mound-initial','native-state-v4')]:
  exp=BASE/'official-texture-experiments-v2'/key/'experiment';meta=json.loads((exp/'views.json').read_text());cases.append((key,exp/'approved-model.blend',exp/rev/'worker.blend',meta['object_names']))
 authority=json.loads((BASE/'mound-ownership-v1/report.json').read_text());cases.append(('all-mound-sites',Path(authority['approval']['member']['model']),BASE/'mound-filled-all-sites-v1/worker.blend',[r['object']for site in authority['records']for r in site['objects']]))
 reports=[]
 for key,source,target,names in cases:
  before=capture(source,names);after=capture(target,names);assert before==after,key
  reports.append(dict(key=key,source=str(source),source_sha256=sha(source),model=str(target),model_sha256=sha(target),objects=len(names),geometry_native_uv_original_packed_images_exact=True,signatures=before));print(key,len(names),'PASS',flush=True)
 write_json(BASE/'state-final-uv-source-guard-v1.json',dict(status='PASS',cases=reports))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
