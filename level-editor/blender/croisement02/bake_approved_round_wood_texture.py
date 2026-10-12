"""Private original-atlas fill from physically sampled visible UV/filter footprints."""
from pathlib import Path
import sys,json,hashlib,shutil
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from array import array
ROOT=Path.cwd();R=Path(__file__).resolve().parent;number=int(sys.argv[sys.argv.index('--')+1]);assert number in (32,38)
sys.path[:0]=[str(ROOT/'level-editor/blender/croisement02'),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tile
from project_reviewed_texture import _read
read=lambda p:json.loads(p.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
exp=ROOT/f'level-editor/work/active/croisement02/approved-round-wood-textures-oct11-v1/tree{number}/experiment';out=exp/'original-atlas-bake-v2';meta=read(exp/'views.json');source=exp/'approved-model.blend';gen=exp/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-preserved.png';maskpath=exp/'mask.png'
assert sha(source)=={32:'3c34560c616fb4d6802ff517aa2fc3ccd5fbb8b0abbce162e4f6481558c8db1f',38:'f4ddf84fe18ca6933301066258bc5e733f6d7ebe4737fa8434b5e6d4c215358a'}[number]
assert shutil.disk_usage(ROOT).free>10*1024**3
protected=np.array(Image.open(maskpath).convert('RGBA'))[:,:,3]>0
inp=np.array(Image.open(exp/'input.png').convert('RGBA'));generated=np.array(Image.open(gen).convert('RGBA'))
assert np.array_equal(inp[protected],generated[protected]);assert np.array_equal(inp[:,:,3],generated[:,:,3])
review=read(exp/'generation-review.json');assert review['ready_for_bake'];assert sha(gen)==review['preserved_sha256']
acquire(slots=2)
try:
 available=int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024
 assert available>=6*1024**3 and shutil.disk_usage(ROOT).free>=10*1024**3
 assert not out.exists();out.mkdir();bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update()
 names=meta['object_names'];outside={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.type=='MESH'and o.name not in names};objects=[scene.objects[n]for n in names];geom={n:_geometry(scene.objects[n])for n in names};uvs={n:{u.name:np.array([tuple(v.uv)for v in u.data])for u in scene.objects[n].data.uv_layers}for n in names};slots={n:[p.material_index for p in scene.objects[n].data.polygons]for n in names}
 authority=read(exp/'source-authority-v2/authority.json');assert authority['status']=='PASS_EXACT_SOURCE_REPRODUCTION' and authority['model_sha256']==sha(source)
 state={};verts=[];triangles=[];owners=[]
 for obj in objects:
  matches=[]
  for slot in {p.material_index for p in obj.data.polygons}:
   mat=obj.data.materials[slot]
   assert mat.get('source_ownership_bake') and mat.get('source_ownership_alpha')=='one=observed,zero=inferred;material remains opaque'
   nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image];assert len(nodes)==1;matches.append(nodes[0])
  assert len(matches)==1;node=matches[0];assert node.interpolation=='Linear';uvname=node.inputs['Vector'].links[0].from_node.uv_map
  image=node.image;proof=authority['records'][obj.name];assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==proof['original_packed_image_sha256'];assert sha(Path(proof['path']))==proof['sha256'];domain=np.load(proof['path'])['ownership'];pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels);pixels=pixels.reshape(image.size[1],image.size[0],4);assert pixels.shape[:2]==domain.shape
  state[obj.name]=dict(image=image,image_name=image.name,original=pixels.copy(),pixels=pixels.copy(),domain=domain,scores=np.zeros(domain.shape,np.float32),direct=np.zeros(domain.shape,bool),touched=np.zeros(domain.shape,bool),uvname=uvname)
  obj.data.calc_loop_triangles();offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices)
  for tri in obj.data.loop_triangles:
   triangles.append(tuple(offset+i for i in tri.vertices));owners.append((obj.name,[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices],[Vector((*obj.data.uv_layers[uvname].data[i].uv,0))for i in tri.loops]))
 tree=BVHTree.FromPolygons(verts,triangles,all_triangles=True);floatgen=_read(gen);view_reports=[]
 # Four actual surface samples per editable image pixel capture narrow triangles.
 # Every changed texel is either physically sampled or a positive Linear-filter
 # neighbour of such a sample. Filter continuation is explicitly inferred.
 for view in meta['views']:
  i=view['index'];m=Matrix(view['camera_matrix_world']);right=m.col[0].to_3d();up=m.col[1].to_3d();toward=m.col[2].to_3d();unit=view['ortho_scale']/384;count=0
  for y,x in np.argwhere(~protected[(i//4)*384:(i//4+1)*384,(i%4)*384:(i%4+1)*384]):
   rgb=floatgen[767-((i//4)*384+y),(i%4)*384+x,:3]
   for ox,oy in ((.25,.25),(.75,.25),(.25,.75),(.75,.75)):
    hit,normal,fid,distance=tree.ray_cast(m.translation+right*((x+ox-192)*unit)+up*((192-y-oy)*unit),-toward)
    if hit is None:continue
    facing=normal.dot(toward)
    if facing<=.02:continue
    name,world,uv=owners[fid];q=barycentric_transform(hit,*world,*uv);s=state[name];height,width=s['domain'].shape;fx=q.x*width-.5;fy=q.y*height-.5;ix=int(np.floor(fx));iy=int(np.floor(fy));nearest=(min(width-1,max(0,int(q.x*width))),min(height-1,max(0,int(q.y*height))))
    if s['domain'][nearest[1],nearest[0]]!=0:continue
    count+=1
    for tx,ty,weight in ((ix,iy,(1-(fx-ix))*(1-(fy-iy))),(ix+1,iy,(fx-ix)*(1-(fy-iy))),(ix,iy+1,(1-(fx-ix))*(fy-iy)),(ix+1,iy+1,(fx-ix)*(fy-iy))):
     if not(0<=tx<width and 0<=ty<height)or weight<=1e-5 or s['domain'][ty,tx]!=0:continue
     score=facing*weight
     s['direct'][ty,tx]|=(tx,ty)==nearest;s['touched'][ty,tx]=True
     if score>s['scores'][ty,tx]:s['scores'][ty,tx]=score;s['pixels'][ty,tx,:3]=rgb
  view_reports.append(dict(view=i,physical_unknown_samples=count));print('view',i,count,flush=True)

 # Complete occupied UV texels on physically visible triangle intersections.
 # Center-outside cells are clipped to the triangle, never presumed padding.
 def clip_cell(poly,x0,y0,x1,y1):
  for axis,edge,greater in ((0,x0,True),(0,x1,False),(1,y0,True),(1,y1,False)):
   result=[]
   for a,b in zip(poly,poly[1:]+poly[:1]):
    ia=a[axis]>=edge if greater else a[axis]<=edge;ib=b[axis]>=edge if greater else b[axis]<=edge
    if ia:result.append(a)
    if ia!=ib:
     t=(edge-a[axis])/(b[axis]-a[axis]);result.append((a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])))
   poly=result
   if len(poly)<3:return None
  area=sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(poly,poly[1:]+poly[:1]))
  if abs(area)<1e-9:return None
  return np.mean(poly,axis=0)
 cameras=[(v,Matrix(v['camera_matrix_world']).inverted(),Matrix(v['camera_matrix_world']).col[2].to_3d())for v in meta['views']]
 projection_hits=0;subpixel_cells=0;candidate_count=0
 for fid,(name,world,uv)in enumerate(owners):
  st=state[name];height,width=st['domain'].shape;points2=np.array([(u.x*width,u.y*height)for u in uv]);a,b,c=points2;den=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
  if abs(den)<1e-10:continue
  normal=(world[1]-world[0]).cross(world[2]-world[0]).normalized();ordered=sorted([(normal.dot(toward),view,inv,toward)for view,inv,toward in cameras if normal.dot(toward)>.02],key=lambda t:-t[0])
  if not ordered:continue
  x0=max(0,int(np.floor(points2[:,0].min())));x1=min(width,int(np.ceil(points2[:,0].max())));y0=max(0,int(np.floor(points2[:,1].min())));y1=min(height,int(np.ceil(points2[:,1].max())))
  for ty in range(y0,y1):
   for tx in range(x0,x1):
    if st['domain'][ty,tx]!=0:continue
    qx,qy=tx+.5,ty+.5;ba=((b[1]-c[1])*(qx-c[0])+(c[0]-b[0])*(qy-c[1]))/den;bb=((c[1]-a[1])*(qx-c[0])+(a[0]-c[0])*(qy-c[1]))/den;bc=1-ba-bb
    if min(ba,bb,bc)<-1e-6:
     clipped=clip_cell([tuple(p)for p in points2],tx,ty,tx+1,ty+1)
     if clipped is None:continue
     qx,qy=clipped;ba=((b[1]-c[1])*(qx-c[0])+(c[0]-b[0])*(qy-c[1]))/den;bb=((c[1]-a[1])*(qx-c[0])+(a[0]-c[0])*(qy-c[1]))/den;bc=1-ba-bb;subpixel_cells+=1
    candidate_count+=1;point=world[0]*float(ba)+world[1]*float(bb)+world[2]*float(bc)
    for facing,view,inv,toward in ordered:
     local=inv@point;px=(.5+local.x/view['ortho_scale'])*384;py=(.5-local.y/view['ortho_scale'])*384;xx=int(np.floor(px));yy=int(np.floor(py))
     if not(0<=xx<384 and 0<=yy<384):continue
     sx=(view['index']%4)*384+xx;sy=(view['index']//4)*384+yy
     if protected[sy,sx]:continue
     hit,nn,fi,dist=tree.ray_cast(point+toward*10000,-toward)
     if hit is None or (hit-point).length>.025:continue
     # Physically sampled texel interior outranks earlier filter continuation.
     st['pixels'][ty,tx,:3]=floatgen[767-sy,sx,:3];st['touched'][ty,tx]=True;st['direct'][ty,tx]=True;projection_hits+=1;break
  if fid%10000==0:print('occupied triangles',fid,'hits',projection_hits,'subpixel',subpixel_cells,flush=True)
 print('occupied coverage',candidate_count,projection_hits,subpixel_cells,flush=True)
 records=[]
 assert all(s['touched'].any() for s in state.values()),'No generated texture was applied'
 for name,s in state.items():
  unchanged=~s['touched'];assert np.array_equal(s['pixels'][unchanged],s['original'][unchanged]);assert np.array_equal(s['pixels'][s['domain']!=0],s['original'][s['domain']!=0]);assert np.array_equal(s['pixels'][:,:,3],s['original'][:,:,3]);s['image'].pixels.foreach_set(s['pixels'].ravel());s['image'].update();s['image'].pack();np.savez_compressed(out/(name.rsplit(' ',1)[-1]+'-coverage.npz'),changed=s['touched'],direct=s['direct']);records.append(dict(object=name,changed_kind0_texels=int(s['touched'].sum()),physically_sampled=int((s['touched']&s['direct']).sum()),linear_filter_continuation=int((s['touched']&~s['direct']).sum())))
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);digest=sha(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene
 for name,s in state.items():
  obj=scene.objects[name];assert _geometry(obj)==geom[name];assert slots[name]==[p.material_index for p in obj.data.polygons]
  for u in obj.data.uv_layers:assert np.array_equal(uvs[name][u.name],np.array([tuple(v.uv)for v in u.data]))
  im=bpy.data.images[s['image_name']];px=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(px);px=px.reshape(s['original'].shape)
  assert np.array_equal(px[~s['touched']],s['original'][~s['touched']]),name+' protected mismatch';assert np.array_equal(px[:,:,3],s['original'][:,:,3]);assert np.array_equal(px[s['domain']!=0],s['original'][s['domain']!=0])
 assert outside=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in outside}
 (out/'preservation.json').write_text(json.dumps(dict(status='PASS',model_sha256=digest,source_model_sha256=sha(source),generated_sha256=sha(gen),geometry_uv_material_slots_exact=True,all_original_alpha_exact=True,observed_rgba_exact=True,outside_appearance_exact=True,untouched_kind0_rgba_exact=True,records=records,views=view_reports,occupied_uv_candidates=candidate_count,occupied_uv_projection_hits=projection_hits,clipped_subpixel_cells=subpixel_cells,scope='Approved wood only. Exact reproduced categorical source provenance protects all observed RGBA. Changed RGB is generated inference, including bounded linear-filter continuation. Original alpha, UVs, geometry and all outside appearances remain exact. Unseen atlas completion is not claimed; saved actual review required.'),indent=2)+'\n')
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(exp/'views.json',out/'actual',width=384);buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);buf=array('f',[0])*len(im.pixels);im.pixels.foreach_get(buf);buffers.append(buf);bpy.data.images.remove(im)
 _tile(buffers,384,384,out/'actual/textured.png');print('PASS',digest,records,flush=True)
finally:release()
