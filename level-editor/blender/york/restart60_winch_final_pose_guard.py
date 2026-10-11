"""Independently reopen approved assembly and final appearance to pin every source pose."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';O=W/(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'restart60-winch-chain-combined-v2');source=W/'restart59-approved-winch-assembly-v1/model.blend';sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest();assert sha(source)=='dd3177aa0639bebadcf99a954f92a3908db2d69a681e2ae48b5592beddbed33f'
assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from refinement_workspace import _geometry
receivers=set(json.loads((W/'restart60-winch-chain-texture-inputs-v1/input-review.json').read_text())['object_names']);stable=set(json.loads((W/'approved-texture-inputs-v1/stable24/input-review.json').read_text())['object_names']);scopes=[];baseline=[]
def records():
 scene=bpy.context.scene;out=[]
 for frame in range(45):
  scene.frame_set(frame*2);bpy.context.view_layer.update();out.append({'frame':frame,'tick':frame*2,'geometry':{o.name:_geometry(o) for o in scene.objects if o.type=='MESH'},'outside_appearance':{o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in receivers},'stable24_appearance':{n:_geometry(scene.objects[n],protect_appearance=True) for n in sorted(stable)}})
 return out
bpy.ops.wm.open_mainfile(filepath=str(source));baseline=records();names=sorted(o.name for o in bpy.context.scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004' and o.name not in stable);assert len(names)==85
bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));final=records();assert final==baseline
report={'status':'PASS','approved_assembly':{'path':str(source),'sha256':sha(source)},'candidate':{'path':str(O/'worker.blend'),'sha256':sha(O/'worker.blend')},'receiver_objects':names,'stable24_objects':sorted(stable),'exact_approved_geometry_and_stable24_appearance_all45':True,'poses':final};(O/'approved-assembly-pose-guard.json').write_text(json.dumps(report,indent=2)+'\n');print({'status':'PASS','sha256':sha(O/'approved-assembly-pose-guard.json'),'model_sha256':sha(O/'worker.blend')})
