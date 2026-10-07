"""Check the entire candidate phase range before fitting closed-loop motion."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-chain-loop-prototype-v7';OUT=BASE/'phase-contacts.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
proposal=json.loads((BASE/'proposal.json').read_text());s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1))
def world(x,y,z):return Vector((x,-y/s,z/c))
left=world(2400.5,1059,104);right=world(2410,1064,104);mid=(left+right)/2;direction=(right-left).normalized();radius=proposal['radius_world'];height=proposal['straight_height'];length=proposal['path_length'];spacing=proposal['spacing_world']
# Evaluate the exact fitting recipe's pose function without executing its sweep.
recipe=Path(__file__).with_name('restart11_winch_closed_loop_fit.py');node=next(n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='pose');exec(compile(ast.Module(body=[node],type_ignores=[]),str(recipe),'exec'))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));links=sorted((o for o in bpy.context.scene.objects if o.name.startswith('Complete chain loop link')),key=lambda o:o.name);idler=bpy.context.scene.objects['Inferred upper chain idler'];inv=idler.matrix_world.inverted();idler_r=max(math.hypot(v.co.x,v.co.y) for v in idler.data.vertices);half=max(abs(v.co.z) for v in idler.data.vertices);count=256;rows=[]
for phase in range(14):
 for i,o in enumerate(links):o.location,o.rotation_euler=pose(i*spacing+(5.5+phase*.5)/c,i)
 bpy.context.view_layer.update();samples=[np.array([tuple(o.matrix_world@Vector((1.5*math.cos(i*math.tau/count),3*math.sin(i*math.tau/count),0))) for i in range(count)]) for o in links];distances=[]
 for a,b in zip(samples,samples[1:]+samples[:1]):distances.append(float(np.sqrt(((a[:,None,:]-b[None,:,:])**2).sum(axis=2).min())))
 lower=min(distances)-2*3*math.pi/count;penetrating=0
 for o in links:
  for v in o.data.vertices:
   p=inv@(o.matrix_world@v.co)
   penetrating+=int(abs(p.z)<half-1e-4 and math.hypot(p.x,p.y)<idler_r-1e-4)
 rows.append({'phase_game':phase*.5,'conservative_link_surface_clearance':lower-.56,'maximum_safe_wire_radius_bound':lower/2,'idler_penetrating_vertices':penetrating})
OUT.write_text(json.dumps({'status':'Conservative sampled phase contacts; all14 fitted phases, not a proof for unsampled continuous-time poses','model_sha256':proposal['model_sha256'],'pose_recipe_sha256':hashlib.sha256(recipe.read_bytes()).hexdigest(),'rows':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
