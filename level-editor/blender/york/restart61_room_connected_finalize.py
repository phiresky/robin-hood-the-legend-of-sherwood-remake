"""Remove zero-area partition fragments and restore explicit canonical source owners."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-interior-v19';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,bmesh
source=W/'restart61-room-interior-v18/model.blend';assert hashlib.sha256(source.read_bytes()).hexdigest()=='c996e9bfc981c0e8727977473ab9459a11b04222e6462046265a91699e6155bd';bpy.ops.wm.open_mainfile(filepath=str(source));rows=[]
for o in bpy.context.scene.objects:
 if o.type!='MESH' or not(o.get('room_receiver') or o.get('state_recipe')=='initial_only' or o.get('source_node')in {'building-764','building-765','building-776','building-779'}):continue
 original=o.name.rsplit(' ',1)[-1] if o.get('state_recipe')=='initial_only' else o.get('source_node');assert original.startswith('building-') or o.get('room_receiver');o['source_node']=original;o['projection_component']='room-cover'if o.get('state_recipe')=='initial_only'else('room-receiver'if o.get('room_receiver')else'room-remainder');b=bmesh.new();b.from_mesh(o.data);remove=[f for f in b.faces if f.calc_area()<1e-8];count=len(remove);bmesh.ops.delete(b,geom=remove,context='FACES_ONLY');wire=[e for e in b.edges if not e.link_faces];bmesh.ops.delete(b,geom=wire,context='EDGES');isolated=[v for v in b.verts if not v.link_faces];bmesh.ops.delete(b,geom=isolated,context='VERTS');b.to_mesh(o.data);b.free();o.data.update();rows.append({'object':o.name,'source_owner':original,'component':o['projection_component'],'removed_zero_area_faces':count,'remaining_zero_area_faces':sum(p.area<1e-8 for p in o.data.polygons)})
assert all(r['remaining_zero_area_faces']==0 for r in rows);OUT.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);(OUT/'report.json').write_text(json.dumps({'status':'CLEANED_GEOMETRY_PENDING_RECHECK','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'scope':'Only zero-area faces/wire vertices removed; source identities normalized to original four owners and disjoint cover/remainder component names. New room solids unchanged.','objects':rows},indent=2)+'\n')
