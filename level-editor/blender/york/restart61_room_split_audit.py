"""Check initial union and applied pixel ownership for the private room split."""
import json,math,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-interior-audit-v13';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));pixels=[(x,y)for y in range(780,1081)for x in range(2350,2470)];origins=[Vector((x+.5,-(y+.5)/s,0))+back*10000 for x,y in pixels]
def rays():
 bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();result=[]
 for origin in origins:
  ok,p,n,f,obj,m=bpy.context.scene.ray_cast(deps,origin,-back);result.append((list(p),obj.name)if ok else None)
 return result
bpy.ops.wm.open_mainfile(filepath=str(W/'restart42-channel-assembly-v1/model.blend'));before=rays();bpy.ops.wm.open_mainfile(filepath=str(W/'restart61-room-interior-v13/model.blend'));initial=rays();covers=[o for o in bpy.context.scene.objects if o.get('state_recipe')=='initial_only'];
for cover in covers:cover.hide_viewport=True
applied=rays();alpha=np.array(Image.open(W.parent/'geometry-pass-01/native-state-source-v1/patch-005/row-0/frame-000.png'))[:,:,3];errors=[];changed=[];owners={}
def difference(a,b):return (a is None)!=(b is None) or (a is not None and b is not None and max(abs(x-y)for x,y in zip(a[0],b[0]))>.005)
for xy,a,b,d in zip(pixels,before,initial,applied):
 if difference(a,b):errors.append(xy)
 if difference(a,d):
  x,y=xy;inside=2356<=x<2435 and 876<=y<1001 and alpha[y-876,x-2356]>0;changed.append({'pixel':xy,'inside_patch_alpha':bool(inside),'applied_owner':d[1]if d else None});key=d[1]if d else'absent';owners[key]=owners.get(key,0)+1
OUT.mkdir();report={'status':'AUDIT_COMPLETE_NOT_APPROVAL','rays':len(pixels),'initial_changed_pixels':len(errors),'initial_errors':errors[:20],'applied_changed_pixels':len(changed),'applied_changed_outside_source_alpha':sum(not r['inside_patch_alpha']for r in changed),'applied_owners':owners,'model_sha256':hashlib.sha256((W/'restart61-room-interior-v13/model.blend').read_bytes()).hexdigest()};(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');(OUT/'changed.json').write_text(json.dumps(changed));print(json.dumps(report))
