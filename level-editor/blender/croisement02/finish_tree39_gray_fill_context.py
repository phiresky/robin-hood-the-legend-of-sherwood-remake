"""Resume context renders after the completed new gray-only input packet."""
import importlib.util
from pathlib import Path
import json,shutil
import bpy
p=Path(__file__).with_name('prepare_tree39_gray_fill.py');spec=importlib.util.spec_from_file_location('prep',p);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
m.acquire(slots=2)
try:
 assert shutil.disk_usage(m.R).free>=10*1024**3
 assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>=6*1024**3
 bpy.ops.wm.open_mainfile(filepath=str(m.OUT/'worker.blend'))
 frames=json.loads((m.OUT/'actual-frames.json').read_text());frames.pop('render_object_names',None);m.dump(m.OUT/'context-frames-v2.json',frames)
 m.render(m.OUT/'context-frames-v2.json',m.OUT/'context',modes=('textured','solid'),width=384);m.tile(m.OUT/'context','textured');m.tile(m.OUT/'context','solid')
 refs=json.loads((m.B/'restart6-five-bark-fill-v1/tree-39-input-v1/auxiliary-references.json').read_text())['references']
 m.dump(m.OUT/'supplementary-references.json',{'references':refs,'own_context':str(m.OUT/'context/textured.png')})
 m.dump(m.OUT/'scope.json',{'status':'INPUT_REVIEW_PENDING_NO_API_REQUEST','live_model':str(m.LIVE),'live_sha256':m.sha(m.LIVE),'input_model':str(m.OUT/'worker.blend'),'input_model_sha256':m.sha(m.OUT/'worker.blend'),'scope':'Only three existing constant-gray wood primitive sets; existing source/materials/crown protected. No geometry changes.','input_packet':str(m.OUT/'packet/views.json'),'supplementary_context':str(m.OUT/'context/textured.png'),'permitted_material_references':refs,'first_view':'Original approved camera at index0/top-left','canonical_writes':False})
finally:m.release()
