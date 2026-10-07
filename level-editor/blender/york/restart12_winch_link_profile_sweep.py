"""Measure coherent oval-link alternatives around the complete inferred return."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-chain-loop-prototype-v4';OUT=BASE/'profile-sweep.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));links=sorted((o for o in bpy.context.scene.objects if o.name.startswith('Complete chain loop link')),key=lambda o:o.name);rows=[];count=256
for rx in (1.2,1.5,1.8):
 for ry in (2.4,2.7,3.0):
  samples=[np.array([tuple(o.matrix_world@Vector((rx*math.cos(i*math.tau/count),ry*math.sin(i*math.tau/count),0))) for i in range(count)]) for o in links];distances=[]
  for a,b in zip(samples,samples[1:]+samples[:1]):distances.append(float(np.sqrt(((a[:,None,:]-b[None,:,:])**2).sum(axis=2).min())))
  lower=min(distances)-2*max(rx,ry)*math.pi/count
  rows.append({'centerline_half_width':rx,'centerline_half_height':ry,'minimum_centerline_distance_bound':lower,'maximum_clear_wire_radius_bound':lower/2,'limiting_adjacent_indices':[i for i,d in enumerate(distances) if d==min(distances)],'straight_run_end_overlap':2*ry-json.loads((BASE/'proposal.json').read_text())['spacing_world']})
OUT.write_text(json.dumps({'status':'Link-profile diagnostic only; no geometry written; linked topology and swept-time contact still unverified','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'rows':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
