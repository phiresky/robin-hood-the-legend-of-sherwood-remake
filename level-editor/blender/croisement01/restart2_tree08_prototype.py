"""Bounded native wood scaffold trial; deliberately excludes crown and terrain."""
import hashlib,heapq,json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';revision=int(sys.argv[sys.argv.index('--revision')+1]) if '--revision' in sys.argv else 1;out=R/f'tree08-wood-prototype-v{revision}';budget=128*1024**2;floor=10*1024**3

def guard(reserve=1024**2):
 used=sum(p.stat().st_size for folder in R.glob('tree08-wood-prototype-v*') for p in folder.rglob('*') if p.is_file())
 assert used+reserve<=budget,(used,reserve,'Output budget')
 assert shutil.disk_usage(R).free>=floor+max(reserve,budget-used),'Disk floor with remaining round reserve'

def save_json(name,data):guard();(out/name).write_text(json.dumps(data,indent=2)+'\n')
guard();out.mkdir(exist_ok=False);acquire();guard();bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.preferences.filepaths.save_version=0
scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
plan=json.loads((R/'tree08-topology-plan-v1/plan.json').read_text());trace=json.loads((R/'tree08-source-trace-v2/trace.json').read_text());selected=[r['trace_id'] for r in plan['path_classification'] if r['structural_scaffold']]
if revision>=2:
 # Restore secondary wood supported by actual missed core pixels, preserving
 # graph paths to the rooted scaffold; no manual blanket crown subtraction.
 miss=json.loads((R/'tree08-wood-prototype-v1/miss-attribution.json').read_text())['nearest_trace_counts'];needed={int(k) for k,v in miss.items() if v>=3};adj={};edge={}
 for i,path in enumerate(trace['polylines']):
  for a,b in zip(path,path[1:]):
   u,v=tuple(a[:2]),tuple(b[:2]);cost=math.dist(u,v);adj.setdefault(u,{})[v]=cost;adj.setdefault(v,{})[u]=cost;edge[frozenset((u,v))]=i
 root=tuple(plan['root_native']);dist={root:0};prev={};q=[(0,root)]
 while q:
  d,u=heapq.heappop(q)
  if d!=dist[u]:continue
  for v,cost in adj[u].items():
   if d+cost<dist.get(v,float('inf')):dist[v]=d+cost;prev[v]=u;heapq.heappush(q,(d+cost,v))
 additions=set()
 for i in needed:
  for endpoint in [trace['polylines'][i][0],trace['polylines'][i][-1]]:
   u=tuple(endpoint[:2])
   if u not in dist:continue
   while u!=root:
    v=prev[u];additions.add(edge[frozenset((u,v))]);u=v
 selected=sorted(set(selected)|additions)
 if revision>=3:
  # A source branch can be a projected graph cycle. Shortest routes to its
  # endpoints must not delete that branch merely because another route exists.
  supported_edges={i for i in needed if tuple(trace['polylines'][i][0][:2]) in dist and tuple(trace['polylines'][i][-1][:2]) in dist}
  selected=sorted(set(selected)|supported_edges)
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));right=Vector((1,0,0))
# Native screen coordinates remain exact. Below the flare, screen-down is
# mainly slope/depth instead of a vertical dangling cylinder. Soil is deferred.
def center(x,y):
 z=(370-y)/c if y<=370 else -(y-370)*.25
 return Vector((x,-(y+z*c)/s,z))
verts=[];faces=[];section_ranges=[];sides=12
for trace_id in selected:
 path=trace['polylines'][trace_id]
 if len(path)<2:continue
 start=len(verts)
 for i,(x,y,r) in enumerate(path):
  before=path[max(0,i-1)];after=path[min(len(path)-1,i+1)];dx,dy=after[0]-before[0],after[1]-before[1];length=max(.001,math.hypot(dx,dy));normal=right*(-dy/length)+down*(dx/length)
  # Circular width/depth section; source radius is a conservative initial
  # silhouette estimate, not a measured physical depth.
  radius=max(.65,r*(1.14 if revision>=2 else 1.08))
  for j in range(sides):
   a=2*math.pi*j/sides;verts.append(center(x,y)+normal*(math.cos(a)*radius)+ray*(math.sin(a)*radius))
 for i in range(len(path)-1):
  for j in range(sides):a=start+i*sides+j;b=start+i*sides+(j+1)%sides;faces.append((a,b,b+sides,a+sides))
 faces.extend([tuple(start+j for j in reversed(range(sides))),tuple(start+(len(path)-1)*sides+j for j in range(sides))]);section_ranges.append(dict(trace_id=trace_id,vertex_start=start,vertex_count=len(path)*sides))
mesh=bpy.data.meshes.new('Connected source scaffold sections');mesh.from_pydata(verts,[],faces);mesh.update();obj=bpy.data.objects.new('Tree08 private wood scaffold - unresolved crossings',mesh);scene.collection.objects.link(obj)
obj['scope']=f'Initial wood geometry hypothesis only;{len(selected)} separate swept source sections, junction overlaps not welded; no crown or terrain';obj['source_node']='scenery-tree08-wood-prototype'
uv=mesh.uv_layers.new(name='Native source projection')
for loop in mesh.loops:
 p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((p.x-331)/446,1-(-p.y*s-p.z*c-11)/461)
image_path=R/'tree08-semantic-source-v1/bark-core-proposal-rgba.png';im=bpy.data.images.load(str(image_path));im.pack();mat=bpy.data.materials.new('Protected native bark cores, neutral unknown');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';mix=n.new('ShaderNodeMixRGB');mix.blend_type='MIX';mix.inputs[1].default_value=(.3,.3,.3,1);em=n.new('ShaderNodeEmission');o=n.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(tex.outputs['Color'],mix.inputs[2]);links.new(mix.outputs[0],em.inputs[0]);links.new(em.outputs[0],o.inputs[0]);mesh.materials.append(mat)
# Coverage is a geometry ray test, not a renderer/AA assertion.
bvh=BVHTree.FromPolygons(verts,faces);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;miss=[];hit_count=0
for yy,xx in np.argwhere(core):
 x,y=int(xx)+331+.5,int(yy)+11+.5;origin=right*x+down*y+ray*2000;hit=bvh.ray_cast(origin,-ray,4000)[0]
 if hit is None:miss.append([int(x-.5),int(y-.5)])
 else:hit_count+=1
save_json('coverage.json',dict(core_pixels=int(core.sum()),covered=hit_count,misses=len(miss),miss_native_pixels=miss,claim='Initial scaffold coverage only; no permission to discard uncovered native wood. Exact known-core texture packed unchanged.',source_rgba_sha256=hashlib.sha256(image_path.read_bytes()).hexdigest()))
# Graph connectivity is distinct from welded physical connectivity.
save_json('construction.json',dict(status='PRIVATE INITIAL PROTOTYPE; not final geometry or contact proof',selected_trace_count=len(selected),vertices=len(verts),faces=len(faces),sections=section_ranges,source_plan_sha256=hashlib.sha256((R/'tree08-topology-plan-v1/plan.json').read_bytes()).hexdigest(),root_terrain_anchors=plan['root_terrain_anchors'],limitations=['Source graph connected; swept sections overlap at intended junctions but are not welded.','Projected crossings8/14 remain hypotheses; no false ownership resolution.','Disconnected tips not joined. Unselected source traces retained externally.','Root depth is initial slope hypothesis without terrain receiver/contact proof.','Only reviewed bark-core RGB is displayed; other pixels remain neutral, no leaf synthesis.','No crown, terrain, gameplay or canonical changes.']))
guard(32*1024**2);bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);assert (out/'model.blend').stat().st_size<=32*1024**2
# Small native-camera diagnostic only; all8 follows coverage/construction review.
scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.device='CPU';scene.render.resolution_x=446;scene.render.resolution_y=461;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.world=bpy.data.worlds.new('Neutral world');scene.world.color=(.08,.08,.08)
camdata=bpy.data.cameras.new('Original game camera');cam=bpy.data.objects.new('Original game camera',camdata);scene.collection.objects.link(cam);camdata.type='ORTHO';camdata.ortho_scale=461;target=right*554+down*241.5;cam.location=target+ray*1500;cam.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler();scene.camera=cam;scene.render.filepath=str(out/'native-prototype.png');guard(4*1024**2);bpy.ops.render.render(write_still=True)
save_json('saved-receipt.json',dict(model_sha256=hashlib.sha256((out/'model.blend').read_bytes()).hexdigest(),model_bytes=(out/'model.blend').stat().st_size,total_bytes=sum(p.stat().st_size for p in out.rglob('*') if p.is_file()),threads=2,coverage=[hit_count,int(core.sum())],canonical_unchanged=True));print('TREE08 PROTOTYPE COMPLETE',hit_count,int(core.sum()),flush=True)
