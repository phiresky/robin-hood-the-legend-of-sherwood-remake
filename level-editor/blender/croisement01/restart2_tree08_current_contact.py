"""Saved Tree08 roots against exact current ground and native terrace exports."""
import argparse,hashlib,json,math,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';out=R/'tree08-current-contact-v2';lib=ROOT/'level-editor/library';model=R/'tree08-wood-prototype-v12-local-junctions/model.blend';cap=32*1024**2
parser=argparse.ArgumentParser();parser.add_argument('--model',type=Path);parser.add_argument('--output',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);model=args.model.resolve() if args.model else model;out=args.output.resolve() if args.output else out
assert not out.exists()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def guard():
 assert shutil.disk_usage(R).free>=10*1024**3+cap
 assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
 assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file())<=cap
guard();acquire();out.mkdir();digest=sha(model);scene_path=lib/'scenes/croisement01.rhlos-map.json';manifest=json.loads(scene_path.read_text());source_hash=sha(scene_path);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;wood=next(o for o in scene.objects if o.type=='MESH');wood.color=(.6,.35,.15,1);support=[];bindings=[];s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));right=Vector((1,0,0))
selected=[manifest['sceneAssets'][0],next(v for v in manifest['assetSources'] if v['id']=='croisement01-terrace-000')]
for asset in selected:
 path=lib/asset['model'];descriptor=lib/asset['descriptor'];assert sha(path)==asset['model_sha256'] and sha(descriptor)==asset['descriptor_sha256'];before=set(scene.objects);bpy.ops.import_scene.gltf(filepath=str(path));new=set(scene.objects)-before;placement=next((p for p in manifest['placements'] if p['assets']==[asset['id']]),None);translation=Vector((0,0,0))
 if placement:
  t=placement['transform'];assert t['rot_deg']==0;translation=Vector((t['dx'],-t['dy']/s,t['dz']/c))
  overrides=placement.get('parts',{});assert set(overrides)<={'terrace-000'}
  if overrides:
   t=overrides['terrace-000']['transform'];assert t['rot_deg']==0;translation+=Vector((t['dx'],-t['dy']/s,t['dz']/c))
 for o in new:
  if o.parent is None:
   if asset.get('role')=='ground':o.matrix_world=Matrix.Rotation(-math.pi/2,4,'X')@o.matrix_world
   o.location+=translation
 bpy.context.view_layer.update()
 for o in new:
  if o.type=='MESH':o.color=(.25,.38,.3,1);support.append(o)
 points=[o.matrix_world@v.co for o in new if o.type=='MESH' for v in o.data.vertices];bounds=[[min(v[k] for v in points) for k in range(3)],[max(v[k] for v in points) for k in range(3)]]
 if asset.get('role')=='ground':assert max(abs(bounds[j][2]) for j in range(2))<.001 and abs(bounds[1][0]-1408)<.001
 else:assert bounds[0][0]<20 and bounds[1][0]>1000 and bounds[0][1]<-850 and bounds[1][2]>36
 bindings.append(dict(asset=asset,translation=list(translation),objects=[o.name for o in new],world_bounds=bounds,placement=placement))
world=[wood.matrix_world@v.co for v in wood.data.vertices];rootpoints=[v for v in world if -v.y*s-v.z*c>350];target=Vector(tuple((min(v[k] for v in rootpoints)+max(v[k] for v in rootpoints))/2 for k in range(3)));camdata=bpy.data.cameras.new('Current receiver root diagnostic');cam=bpy.data.objects.new(camdata.name,camdata);scene.collection.objects.link(cam);scene.camera=cam;camdata.type='ORTHO';camdata.clip_end=10000;directions=[Vector((c*math.sin(i*math.tau/8),-c*math.cos(i*math.tau/8),s)) for i in range(8)];extent=max(max(abs((p-target).dot(d.cross(Vector((0,0,1))).normalized())),abs((p-target).dot(d.cross(Vector((0,0,1))).normalized().cross(d)))) for p in rootpoints for d in directions);camdata.ortho_scale=extent*2*1.3
scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.film_transparent=False;scene.world=bpy.data.worlds.new('Receiver review world');scene.world.color=(.12,.12,.12);scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=256
for mode in ['actual','solid']:
 folder=out/mode;folder.mkdir();scene.render.engine='CYCLES' if mode=='actual' else 'BLENDER_WORKBENCH';scene.display.shading.color_type='OBJECT';scene.display.shading.light='STUDIO';scene.display.shading.show_cavity=True;sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
 for i,d in enumerate(directions):
  guard();cam.location=target+d*1800;cam.rotation_euler=(-d).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(folder/f'{i}.png');bpy.ops.render.render(write_still=True);sheet.paste(Image.open(folder/f'{i}.png').convert('RGB'),(384*(i%4),408*(i//4)+24));draw.text((384*(i%4)+4,408*(i//4)+5),f'{mode} roots {i}',fill='white')
 sheet.save(folder/'sheet.png')
trees=[(o.name,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in support];woodtree=BVHTree.FromPolygons(world,[list(p.vertices) for p in wood.data.polygons]);contacts=[]
for a in json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_terrain_anchors']:
 x,y=a['native'];hit=woodtree.ray_cast(right*x+down*y+ray*2000,-ray,4000)[0];assert hit is not None;surfaces=[]
 for name,tree in trees:
  surface=tree.ray_cast(Vector((hit.x,hit.y,2000)),Vector((0,0,-1)),4000)[0]
  if surface is not None:surfaces.append(dict(receiver=name,surface=list(surface),wood_z_minus_surface_z=hit.z-surface.z))
 contacts.append(dict(anchor=a,wood=list(hit),vertical_supports=surfaces))
routes=[]
for route in json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['structural_routes']:
 if route['name'] not in ['left basal root','descending root']:continue
 for x,y in route['source_path']:
  if not 340<=y<=471:continue
  origin=right*(x+.5)+down*(y+.5)+ray*2000;hit=woodtree.ray_cast(origin,-ray,4000)[0];hits=[tree.ray_cast(origin,-ray,4000)[0] for _,tree in trees];hits=[p for p in hits if p is not None];depth=max((p.dot(ray) for p in hits),default=None);routes.append(dict(native=[x,y],route=route['name'],wood_hit=None if hit is None else list(hit),ray_clearance=None if hit is None or depth is None else hit.dot(ray)-depth))
(out/'root-route-visibility.json').write_text(json.dumps(dict(model_sha256=digest,scene_sha256=source_hash,samples=routes,missing=[v for v in routes if v['wood_hit'] is None],occluded=[v for v in routes if v['ray_clearance'] is not None and v['ray_clearance']<=0],scope='Independent traced lower-root routes, not upper bark core; no source texture ownership implied.'),indent=2)+'\n')
assert sha(model)==digest and sha(scene_path)==source_hash;guard();(out/'receipt.json').write_text(json.dumps(dict(model_sha256=digest,scene_sha256=source_hash,bindings=bindings,target=list(target),ortho_scale=camdata.ortho_scale,contacts=contacts,scope='Exact live terrain/terrace000 exports, no receiver or wood changes. Native anchor surface rays only; not full contact clearance proof.'),indent=2)+'\n');print('CURRENT CONTACT DONE',flush=True)
