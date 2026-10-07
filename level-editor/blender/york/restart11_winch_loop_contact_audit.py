"""Check sampled link clearances and actual chain vertices inside the inferred idler."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-chain-loop-prototype-v6';OUT=BASE/'link-contact-audit.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();links=sorted((o for o in scene.objects if o.name.startswith('Complete chain loop link')),key=lambda o:o.name);samples={}
for o in links:samples[o.name]=np.array([tuple(o.matrix_world@Vector((1.8*math.cos(i*math.tau/512),3*math.sin(i*math.tau/512),0))) for i in range(512)])
rows=[]
for a,b in zip(links,links[1:]+links[:1]):
 pa,pb=samples[a.name],samples[b.name];distance=float(np.sqrt(((pa[:,None,:]-pb[None,:,:])**2).sum(axis=2).min()));rows.append({'a':a.name,'b':b.name,'sampled_centerline_distance':distance,'conservative_surface_clearance_lower_bound':distance-.74-2*3*math.pi/512})
idler=scene.objects['Inferred upper chain idler'];inv=idler.matrix_world.inverted();rad=max(math.hypot(v.co.x,v.co.y) for v in idler.data.vertices);half=max(abs(v.co.z) for v in idler.data.vertices);penetrations=[]
for o in links:
 inside=[]
 for v in o.data.vertices:
  local=inv@(o.matrix_world@v.co);r=math.hypot(local.x,local.y)
  if abs(local.z)<half-1e-4 and r<rad-1e-4:inside.append(rad-r)
 if inside:penetrations.append({'link':o.name,'vertices_inside_idler':len(inside),'maximum_radial_penetration':max(inside)})
report={'status':'Geometric diagnostic, sampled circle-wire bound is not exact mesh topology proof','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'wire_diameter':.74,'sampling':512,'adjacent_links':rows,'idler_vertex_penetrations':penetrations};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'minimum_conservative_link_clearance':min(r['conservative_surface_clearance_lower_bound'] for r in rows),'idler_penetrations':penetrations},indent=2))
