"""Read-only identify protected receivers currently covering the room patch domain."""
import json,math,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-rays-v1';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image
source=json.loads((W/'restart61-room-source-v1/source.json').read_text());rgba=np.array(Image.open(ROOT/source['frame_path']));base=W/'restart42-channel-assembly-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene;bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));counts={};hits=[]
for y,x in zip(*np.where(rgba[:,:,3]>0)):
 origin=Vector((2356+float(x)+.5,-(876+float(y)+.5)/s,0))+back*10000;ok,p,n,face,obj,mat=scene.ray_cast(deps,origin,-back)
 if ok:
  key=obj.name;row=counts.setdefault(key,{'source_node':obj.get('source_node'),'pixels':0,'faces':{}});row['pixels']+=1;row['faces'][str(face)]=row['faces'].get(str(face),0)+1;hits.append({'pixel':[int(x),int(y)],'object':key,'face':face,'point':list(p)})
 else:counts.setdefault('uncovered',{'pixels':0})['pixels']+=1
OUT.mkdir();(OUT/'hits.json').write_text(json.dumps(hits));(OUT/'report.json').write_text(json.dumps({'status':'PROTECTED_STATIC_OWNERSHIP_DIAGNOSIS','source_pixels':len(hits),'static_model_sha256':hashlib.sha256(base.read_bytes()).hexdigest(),'owners':counts},indent=2)+'\n');print(json.dumps(counts,indent=2))
