"""Recheck unchanged wagon supports and render filled ground contacts."""
import sys,shutil,json,hashlib
import bpy
import numpy as np
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import restart4_south_initial_wagon as wagon
from catalog import OUT
wagon.DEST=OUT/'restart4-south-cart-texture/approved-fill-v1/experiment/native-retained-v1'
if '--candidate' in sys.argv:wagon.DEST=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve()
wagon.VARIANT='v8';wagon.AUDIT='current-support-v1'
for role in ('roof','fore_platform','cabin_walls'):
 shutil.copyfile(OUT/f'restart3-south-cart/initial-physical-v8/{role}-source.png',wagon.DEST/f'{role}-source.png')
from render_slots import acquire,release
acquire()
try:
    E=wagon.DEST.parent
    bpy.ops.wm.open_mainfile(filepath=str(E/'approved-model.blend'))
    uv={o.name:{u.name:np.array([d.uv[:] for d in u.data]) for u in o.data.uv_layers} for o in bpy.context.scene.objects if o.type=='MESH'}
    bpy.ops.wm.open_mainfile(filepath=str(wagon.DEST/'worker.blend'))
    for name,layers in uv.items():
        for key,expected in layers.items():
            actual=np.array([d.uv[:] for d in bpy.context.scene.objects[name].data.uv_layers[key].data])
            assert np.array_equal(expected,actual), (name,key)
    (wagon.DEST/'native-uv-audit.json').write_text(json.dumps(dict(status='PASS',model_sha256=hashlib.sha256((wagon.DEST/'worker.blend').read_bytes()).hexdigest(),approved_preparation_sha256=hashlib.sha256((E/'approved-model.blend').read_bytes()).hexdigest(),all_original_uv_coordinates_exact=True,objects=len(uv)),indent=2)+'\n')
    (wagon.DEST/'manifest.json').write_text(json.dumps(dict(model_sha256=hashlib.sha256((wagon.DEST/'worker.blend').read_bytes()).hexdigest(),scope='Texture-only derivative of approved initial wagon; exact geometry unchanged'),indent=2)+'\n')
    wagon.support()
finally:release()
