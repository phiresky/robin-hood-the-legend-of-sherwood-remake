"""Deferred read-only Blender diagnostic; full native alpha versus saved geometry.

Run through the repository heavy-job queue, never directly from CPU preparation:
blender --background --python this.py -- --model /absolute/worker.blend --output /new/folder
No model save, rendering, source-domain edits or geometry corrections are performed.
"""
import argparse,hashlib,json,math,sys,shutil
from pathlib import Path
R=Path(__file__).resolve().parents[3];W=R/'level-editor/work/active/croisement03/restart2'
OMIT=('Tree01 observed native frame0 provisional cells','Tree01 observed native proposed static samples')
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 import bpy,numpy as np
 from PIL import Image
 from mathutils import Vector
 from mathutils.bvhtree import BVHTree
 from mathutils.geometry import barycentric_transform
 sys.path.insert(0,str(R/'level-editor/refinement'))
 from render_slots import acquire,release
 p=argparse.ArgumentParser();p.add_argument('--model',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);a.model=a.model.resolve();a.output=a.output.resolve();assert a.model.is_file() and not a.output.exists();pins={};S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=Vector((0,-C,S));epsilon=1e-5
 def pin(path):pins[str(path)]=sha(path);return path
 placement=json.loads(pin(W/'tree01-full-native-placement-v2/receipt.json').read_text());source=json.loads(pin(W/'tree01-bark-completeness-v2/receipt.json').read_text());pin(Path(__file__).resolve());pin(R/'level-editor/refinement/render_slots.py')
 for name,digest in placement['pins'].items():
  path=Path(name);assert sha(path)==digest,('placement input drift',name);pins[name]=digest
 frames=[]
 for row in source['raw_frame_conversion_proof']:
  rawpath=pin(Path(row['raw_path']));normalpath=pin(Path(row['normalized_path']));assert sha(rawpath)==row['raw_sha256'] and sha(normalpath)==row['normalized_sha256'];raw=np.array(Image.open(rawpath).convert('RGBA'));normal=np.array(Image.open(normalpath).convert('RGBA'));key=np.all(raw[:,:,:3]==[0,251,0],axis=2);raw[key]=0;assert np.array_equal(raw,normal);assert row['native_source_box']==[0,0,456,265] and row['map_offset_xy']==[0,0];frames.append(normal)
 assert len(frames)==14;union=np.logical_or.reduce([im[:,:,3]>0 for im in frames]);modelhash=sha(pin(a.model))
 def resources():
  memory={line.split(':')[0]:int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines()};assert memory['MemAvailable']>=6*1024**3,'Need6GiB available memory';assert shutil.disk_usage(R).free>=10*1024**3,'Need10GiB free disk'
 resources();acquire(slots=2)
 try:
  resources()
  bpy.ops.wm.open_mainfile(filepath=str(a.model));scene=bpy.context.scene
  def fingerprint(o):
   m=o.data;return hashlib.sha256(json.dumps({'matrix':[list(v)for v in o.matrix_world],'vertices':[list(v.co)for v in m.vertices],'polygons':[list(p.vertices)for p in m.polygons],'material_indices':[p.material_index for p in m.polygons]},separators=(',',':')).encode()).hexdigest()
  before={o.name:fingerprint(o)for o in scene.objects if o.type=='MESH'};omitted=[]
  for name in OMIT:
   obj=scene.objects.get(name);assert obj is not None,('required diagnostic object missing',name);omitted.append({'name':name,'geometry_sha256':before[name]}) # Omit only from in-memory ray receiver list; leave the object intact.
  assert scene.objects.get('Tree01 inferred crown backs only')is not None
  textures={};material_records={}
  def opacity_program(mat):
   assert mat and mat.use_nodes,('unsupported non-node material',getattr(mat,'name',None));nodes=mat.node_tree.nodes;outputs=[n for n in nodes if n.type=='OUTPUT_MATERIAL'and n.is_active_output];assert len(outputs)==1
   def scalar(sock):
    if not sock.is_linked:return ('constant',float(sock.default_value))
    assert len(sock.links)==1;n=sock.links[0].from_node;name=sock.links[0].from_socket.name
    if n.type=='NEW_GEOMETRY'and name=='Backfacing':return ('back',)
    if n.type=='LIGHT_PATH'and name=='Is Camera Ray':return ('constant',1.) # This diagnostic traces primary native-camera rays only.
    if n.type=='TEX_IMAGE'and name=='Alpha':
     assert n.image and n.interpolation=='Closest'and n.extension in ('CLIP','REPEAT','EXTEND'),('unsupported texture sampler',mat.name,n.name);im=n.image
     if im.name not in textures:
      data=np.asarray(im.pixels[:],np.float32).reshape(im.size[1],im.size[0],4);textures[im.name]={'pixels':data,'pixel_float_sha256':hashlib.sha256(data.tobytes()).hexdigest(),'packed_sha256':hashlib.sha256(bytes(im.packed_file.data)).hexdigest()if im.packed_file else None}
     uvname=None
     if n.inputs['Vector'].is_linked:
      upstream=n.inputs['Vector'].links[0].from_node;assert upstream.type=='UVMAP',('unsupported texture coordinates',mat.name,upstream.type);uvname=upstream.uv_map
     return ('texture',im.name,n.extension,uvname)
    raise AssertionError(('unsupported opacity scalar',mat.name,n.type,name))
   def shader(sock):
    assert sock.is_linked and len(sock.links)==1;n=sock.links[0].from_node
    if n.type=='BSDF_TRANSPARENT':return ('constant',0.)
    if n.type in ('EMISSION','BSDF_DIFFUSE'):return ('constant',1.)
    if n.type=='BSDF_PRINCIPLED':return scalar(n.inputs['Alpha'])
    if n.type=='MIX_SHADER':
     left,right=shader(n.inputs[1]),shader(n.inputs[2]);return left if left==right else ('mix',scalar(n.inputs[0]),left,right)
    raise AssertionError(('unsupported surface shader',mat.name,n.type))
   program=shader(outputs[0].inputs['Surface']);material_records[mat.name]={'program':program,'raster_backface_culling':bool(mat.use_backface_culling)};return program,bool(mat.use_backface_culling)
  rows=[]
  for o in scene.objects:
   if o.type!='MESH'or o.name in OMIT:continue
   assert not any(m.show_render for m in o.modifiers),('unhandled evaluated modifier',o.name)
   m=o.data;m.calc_loop_triangles();v=[o.matrix_world@p.co for p in m.vertices];tri=list(m.loop_triangles);programs=[opacity_program(mat)for mat in m.materials];assert programs,('missing materials',o.name)
   rows.append({'name':o.name,'original_hide_render':bool(o.hide_render),'vertices':v,'triangles':tri,'mesh':m,'tree':BVHTree.FromPolygons(v,[list(t.vertices)for t in tri],all_triangles=True),'programs':programs})
  assert any(r['name'].startswith('Provisional Tree01 wood')for r in rows)
  def opacity(program,row,tri,hit,back):
   kind=program[0]
   if kind=='constant':return program[1]
   if kind=='back':return float(back)
   if kind=='mix':
    f=opacity(program[1],row,tri,hit,back);return (1-f)*opacity(program[2],row,tri,hit,back)+f*opacity(program[3],row,tri,hit,back)
   assert kind=='texture';_,name,extension,uvname=program;uvlayer=row['mesh'].uv_layers.get(uvname)if uvname else row['mesh'].uv_layers.active;assert uvlayer is not None
   uv=barycentric_transform(hit,*[row['vertices'][j]for j in tri.vertices],*[Vector((*uvlayer.data[j].uv,0))for j in tri.loops]);u,v=float(uv.x),float(uv.y);data=textures[name]['pixels']
   if extension=='CLIP'and not(0<=u<=1 and 0<=v<=1):return 0.
   if extension=='REPEAT':u%=1;v%=1
   x=min(max(int(u*data.shape[1]),0),data.shape[1]-1);y=min(max(int(v*data.shape[0]),0),data.shape[0]-1);return float(data[y,x,3])
  # Receiver geometry and native plane do not vary with phase. Cache every UNION pixel,
  # then emit every actual-alpha pixel conflict in each of the fourteen full frames.
  cached={};max_steps=0;coplanar=[];fallbacks=[]
  for row in rows:
   xyz=np.asarray([list(v)for v in row['vertices']],dtype=np.float64)[np.asarray([list(t.vertices)for t in row['triangles']],dtype=int)]
   projected=np.stack((xyz[:,:,0],-xyz[:,:,1]*S-xyz[:,:,2]*C),axis=2)
   row['exact_xyz']=xyz;row['projected_lo']=projected.min(1);row['projected_hi']=projected.max(1)
  def exact_hits(row,origin,x,y):
   # Float32 BVH advancement may land on the same triangle repeatedly. Re-test
   # every projected candidate using float64, retaining distinct coincident faces.
   exact_ray=np.array([0.,-C,S]);origin=np.array([x+.5,-300/S,(300-y-.5)/C])+exact_ray*5000
   xy=np.array([x+.5,y+.5]);ids=np.flatnonzero(np.all(row['projected_lo']<=xy+1e-6,axis=1)&np.all(row['projected_hi']>=xy-1e-6,axis=1))
   triangles=row['exact_xyz'][ids];e1=triangles[:,1]-triangles[:,0];e2=triangles[:,2]-triangles[:,0];direction=-exact_ray
   h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);valid=np.abs(det)>1e-12;inverse=np.divide(1.,det,out=np.zeros_like(det),where=valid)
   offset=np.asarray(origin,dtype=np.float64)-triangles[:,0];u=inverse*np.einsum('ij,ij->i',offset,h);q=np.cross(offset,e1);v=inverse*np.einsum('j,ij->i',direction,q);distance=inverse*np.einsum('ij,ij->i',e2,q)
   selected=np.flatnonzero(valid&(u>=-1e-9)&(v>=-1e-9)&(u+v<=1+1e-9)&(distance>=0)&(distance<=5000+epsilon))
   result=[]
   for j in selected[np.argsort(distance[selected])]:
    face=int(ids[j]);normal=np.cross(e1[j],e2[j]);back=float(np.dot(normal,exact_ray))<=0;hit=Vector(np.asarray(origin)+direction*distance[j]);result.append((face,hit,back,float(distance[j])))
   return result
  for y,x in np.argwhere(union):
   card=Vector((float(x)+.5,-300/S,(300-float(y)-.5)/C));origin=card+ray*5000;hits=[]
   for row in rows:
    probe=origin.copy();visited=set()
    for step in range(4096):
     hit,normal,face,distance=row['tree'].ray_cast(probe,-ray);max_steps=max(max_steps,step)
     if hit is None:break
     absolute=float((origin-hit).dot(ray))
     if absolute>5000+epsilon:break
     if face in visited:
      fallbacks.append({'object':row['name'],'source_xy':[int(x),int(y)],'repeated_triangle':face})
      for exact_face,exact_hit,exact_back,exact_distance in exact_hits(row,origin,int(x),int(y)):
       exact_tri=row['triangles'][exact_face];exact_program,exact_cull=row['programs'][exact_tri.material_index];exact_alpha=0. if exact_cull and exact_back else opacity(exact_program,row,exact_tri,exact_hit,exact_back)
       assert math.isfinite(exact_alpha) and 0<=exact_alpha<=1
       if exact_alpha>0:
        evidence={'object':row['name'],'original_hide_render':row['original_hide_render'],'triangle':exact_face,'material':row['mesh'].materials[exact_tri.material_index].name,'world':list(exact_hit),'alpha':exact_alpha,'backfacing':bool(exact_back),'card_distance_minus_hit_distance':5000-exact_distance,'intersection_method':'float64_all_projected_candidates'}
        if abs(exact_distance-5000)<=epsilon:coplanar.append({'source_xy':[int(x),int(y)],**evidence})
        hits.append(evidence);break
      break
     visited.add(face)
     tri=row['triangles'][face];program,cull=row['programs'][tri.material_index];back=normal.dot(ray)<=0;alpha=0. if cull and back else opacity(program,row,tri,hit,back)
     assert math.isfinite(alpha) and 0<=alpha<=1,('invalid opacity',row['name'],alpha)
     if alpha>0:
      evidence={'object':row['name'],'original_hide_render':row['original_hide_render'],'triangle':face,'material':row['mesh'].materials[tri.material_index].name,'world':list(hit),'alpha':alpha,'backfacing':bool(back),'card_distance_minus_hit_distance':5000-absolute}
      if abs(absolute-5000)<=epsilon:coplanar.append({'source_xy':[int(x),int(y)],**evidence})
      hits.append(evidence);break
     probe=hit-ray*epsilon
    else:raise AssertionError(('alpha traversal exhausted',row['name'],int(x),int(y)))
   hits.sort(key=lambda h:-h['card_distance_minus_hit_distance']);cached[(int(x),int(y))]=hits
  phase_reports=[];outputs={};shared=[{'source_xy':list(xy),'hits':hits}for xy,hits in cached.items()if hits];shared_index={tuple(r['source_xy']):i for i,r in enumerate(shared)};outputs['shared-conflict-hits.json']=json.dumps(shared,separators=(',',':'))+'\n'
  for i,im in enumerate(frames):
   conflicts=[]
   for y,x in np.argwhere(im[:,:,3]>0):
    hits=cached[(int(x),int(y))]
    if hits:conflicts.append([int(x),int(y),im[y,x].tolist(),shared_index[(int(x),int(y))]])
   packet={'phase':i,'native_actual_alpha_pixels':int((im[:,:,3]>0).sum()),'conflicts':conflicts,'conflict_pixels':len(conflicts),'render_visible_conflict_pixels':sum(any(not h['original_hide_render']for h in shared[q[3]]['hits'])for q in conflicts)};outputs[f'phase-{i:02}.json']=json.dumps(packet,separators=(',',':'))+'\n';phase_reports.append({k:v for k,v in packet.items()if k!='conflicts'})
  after={o.name:fingerprint(o)for o in scene.objects if o.type=='MESH'};assert before==after and sha(a.model)==modelhash
  report={'status':'HOLD_EDITOR_PLANE_CONFLICTS'if any(p['conflict_pixels']for p in phase_reports)else'PASS_SCOPED_EDITOR_PLANE_NO_CONFLICTS','pins':pins,'model_sha256':modelhash,'blender_version':bpy.app.version_string,'numpy_version':np.__version__,'omitted_in_ray_test_only':omitted,'all_mesh_geometry_material_indices_transforms_unchanged':True,'geometry_fingerprints':before,'tested_objects':[{'name':r['name'],'original_hide_render':r['original_hide_render'],'triangles':len(r['triangles'])}for r in rows],'materials':material_records,'textures':{k:{key:value for key,value in v.items()if key!='pixels'}for k,v in textures.items()},'phases':phase_reports,'union_pixels_tested':len(cached),'max_alpha_traversal_steps':max_steps,'float64_fallback_count':len(fallbacks),'float64_fallbacks':fallbacks,'ray_step_and_plane_tolerance_world':epsilon,'coplanar_ambiguities':coplanar,'conflict_encoding':'Each phase conflict=[sourceX,sourceY,nativeRGBA,shared-conflict-hits.json index]; shared records retain all per-object firsthits. No phase or pixel evidence discarded.','native_card_world_y':-300/S,'limits':['Exact full shared native alpha at current editor plane300; primary-camera opacity only (Is Camera Ray=1), not secondary rays, display-polyline sorting proof or physical tree ownership.','All saved mesh receivers included even if hidden; original visibility recorded to distinguish diagnostic context conflicts. Missing neighboring scene geometry is not certified.','Two observed cell objects omitted only from ray receiver list; no saved or in-memory mesh edits. Inferred crown, wood and native pixels retained exactly.','Partial nonzero opacity counts conservatively as conflict; unsupported materials/modifiers fail explicitly. No alpha-mask complement or nearest-color ownership inference.','Each affected source pixel reported for every phase; each object contributes its nearest nontransparent hit before/touching native card.','Any conflict is a placement/runtime diagnostic; never delete native pixels or distort geometry merely to satisfy this plane.']};outputs['receipt.json']=json.dumps(report,indent=2)+'\n';total=sum(len(v.encode())for v in outputs.values());assert total<=32*1024**2,('output budget exceeded before writing',total);a.output.mkdir()
  for name,contents in outputs.items():(a.output/name).write_text(contents)
  assert sum(p.stat().st_size for p in a.output.iterdir())<=32*1024**2
  print(json.dumps({'status':report['status'],'phases':phase_reports}));assert sha(a.model)==modelhash
 finally:release()
if __name__=='__main__':main()
