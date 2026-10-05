"""Bind texture receiver polygons to the exact approved cut-end caps only."""
import sys,json,hashlib
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from catalog import OUT
p=OUT/'restart2-state/cleared-fence-inputs-v2';model=p/'model.blend';digest=hashlib.sha256(model.read_bytes()).hexdigest();expected=json.loads((p/'derivation.json').read_text())['prepared_model_sha256']
if digest!=expected:raise ValueError('Prepared model changed')
bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();mapping={};triangles=0
for obj in scene.objects:
 if obj.type!='MESH' or obj.get('asset_group')!='croisement02-south-field-wattle-fence-cleared-state':continue
 faces=[]
 for face in obj.data.polygons:
  xs=[(obj.matrix_world@obj.data.vertices[i].co).x for i in face.vertices]
  if any(max(abs(x-cut)for x in xs)<.002 for cut in [1018,1170]):faces.append(face.index)
 obj.data.calc_loop_triangles();triangles+=sum(t.polygon_index in faces for t in obj.data.loop_triangles);mapping[obj.name]=faces
if len(mapping)!=2 or triangles!=88:raise ValueError('Cut-cap scope changed')
result={'model_sha256':digest,'texture_receiver_face_indices':mapping,'triangles':triangles,'rule':'All polygon vertices lie on approved cut planes x1018 or1170 within.002 world units; all other stored faces protected.','scope':'Only existing cap faces; no geometry or UV change'};(p/'cap-face-receivers.json').write_text(json.dumps(result,indent=2)+'\n');print({k:len(v)for k,v in mapping.items()})
