"""Private native-source reconstruction of the market well and its timber roof."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import _tree
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-v3';SOURCE=B/'grounding/york-grounded.blend'
ASSET='york-market-roofed-stone-well';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S));BOX=(202,1138,260,1212)
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def sig(o):return hashlib.sha256(json.dumps(dict(vertices=[list(v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],uvs=[[list(d.uv)for d in u.data]for u in o.data.uv_layers],matrix=[list(r)for r in o.matrix_world]),sort_keys=True).encode()).hexdigest()
def main():
 D.mkdir(exist_ok=False);source_hash=sha(SOURCE);bpy.ops.wm.open_mainfile(filepath=str(SOURCE));bpy.context.view_layer.update()
 keep=[o for o in bpy.data.collections['york Working'].all_objects if o.type=='MESH'and o.get('source_node')in ['building-035','building-036','building-037','building-086']]
 oldset=[o for o in keep if o.get('asset_group')==ASSET];context=[o for o in keep if o not in oldset];before={o.name:sig(o)for o in context};props={k:oldset[0][k]for k in oldset[0].keys()}
 scene=bpy.data.scenes.new('York market well private');bpy.context.window.scene=scene;protected=set(keep)
 for o in keep:
  p=o.parent
  while p is not None:protected.add(p);p=p.parent
 for o in protected:scene.collection.objects.link(o);o.hide_render=False
 bpy.context.view_layer.update();assert all(sig(o)==before[o.name]for o in context)
 for o in list(bpy.data.objects):
  if o not in protected or o in oldset:bpy.data.objects.remove(o,do_unlink=True)
 level=json.loads((B/'baseline/york.rhp.json').read_text());art=Image.open(B/'baseline/covered.png').convert('RGBA');domain=np.zeros((BOX[3]-BOX[1],BOX[2]-BOX[0]),dtype=np.uint8)
 for i in [49,50]:
  m=level['masks'][i];a=np.array(Image.open(B/f'baseline/masks/{i:06d}.png').convert('L'));x,y=m['box_top_left'];xx=x-BOX[0];yy=y-BOX[1];domain[yy:yy+a.shape[0],xx:xx+a.shape[1]]=np.maximum(domain[yy:yy+a.shape[0],xx:xx+a.shape[1]],a)
 bucket_domain=Image.new('L',(BOX[2]-BOX[0],BOX[3]-BOX[1]));bucket_polygon=[(245,1202),(248,1202),(250,1204),(250,1206),(249,1209),(247,1211),(244,1210),(242,1207),(242,1204)];ImageDraw.Draw(bucket_domain).polygon([(x-BOX[0],y-BOX[1])for x,y in bucket_polygon],fill=255);bucket_domain.save(D/'bucket-inferred-domain.png');domain=np.maximum(domain,np.array(bucket_domain));Image.fromarray(domain).save(D/'native-domain.png')
 def material(name,image=None):
  m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;l=m.node_tree.links;n.clear();out=n.new('ShaderNodeOutputMaterial');p=n.new('ShaderNodeBsdfPrincipled');p.inputs['Base Color'].default_value=(0,0,0,1);p.inputs['Emission Color'].default_value=(.24,.24,.24,1);p.inputs['Emission Strength'].default_value=1;l.new(p.outputs['BSDF'],out.inputs['Surface']);m['source_ownership_fill']='neutral'
  if image:
   uv=n.new('ShaderNodeUVMap');uv.uv_map='NativeSource';t=n.new('ShaderNodeTexImage');t.image=image;t.interpolation='Closest';t.extension='CLIP';l.new(uv.outputs['UV'],t.inputs['Vector']);mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.24,.24,.24,1);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(t.outputs['Color'],mix.inputs[2]);l.new(mix.outputs['Color'],p.inputs['Emission Color'])
  return m
 gray=material('Well inferred gray');own=[]
 def mesh(name,verts,faces,index):
  me=bpy.data.meshes.new(name);me.from_pydata(verts,[],faces);me.update();bm=bmesh.new();bm.from_mesh(me);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(me);bm.free();o=bpy.data.objects.new(name,me);scene.collection.objects.link(o)
  for k,v in props.items():o[k]=v
  o['source_node']=f'building-{index:03d}';o['source_obstacle']=f'building-{index:03d}';o['projection_component']=name.split(' / ')[-1];o['refinement_recipe']='york/restart7_market_well_bucket_candidate.py';o['asset_group']=ASSET;me.materials.append(gray);me.materials.append(gray);uv=me.uv_layers.new(name='NativeSource')
  for p in me.polygons:
   p.material_index=0 if p.normal.dot(RAY)>.05 else 1
   for li in p.loop_indices:
    v=me.vertices[me.loops[li].vertex_index].co;uv.data[li].uv=((v.x-BOX[0])/(BOX[2]-BOX[0]),1-(-v.y*S-v.z*C-BOX[1])/(BOX[3]-BOX[1]))
  own.append(o);return o
 def game(x,y,z):return Vector((x,-y/S,z/C))
 # A closed annular masonry wall, rather than a solid filled obstacle proxy.
 floor=109.751;cx,cy=230.8,-2245.0;n=32;vertices=[]
 for radius,yradius,z in [(18.8,16.7,floor),(19.3,19.5,126.5),(19.8,20.0,129.5),(13.8,14.0,129.5),(13.8,12.2,floor)]:
  for i in range(n):
   a=2*math.pi*i/n;rough=.22*math.sin(5*a)+.15*math.cos(7*a);vertices.append((cx+(radius+rough)*math.cos(a),cy+(yradius+rough)*math.sin(a),z))
 faces=[]
 for band in range(4):
  for i in range(n):j=(i+1)%n;faces.append((band*n+i,band*n+j,(band+1)*n+j,(band+1)*n+i))
 for i in range(n):j=(i+1)%n;faces.append((4*n+i,4*n+j,j,i))
 mesh('Market well / closed stone ring',vertices,faces,35)
 L=game(221.85,1278.59,136.667);R=game(240.9,1297.13,136.667);A=game(205.28772,1284.7657,129.76201);E=game(221.52278,1303.3416,129.76201);T=game(238.34554,1273.0554,127.382);U=game(257.47012,1291.5675,127.382)
 def roof(name,points,index):
  v=list(points)+[p-Vector((0,0,3.2))for p in points];return mesh(name,v,[(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)],index)
 roof('Market well / front pitched roof',[A,E,R,L],36);roof('Market well / rear pitched roof',[L,R,U,T],37)
 axis=(R-L);axis.z=0;axis.normalize();cross=Vector((-axis.y,axis.x,0))
 def post(name,center,low,high,width):
  xy=[Vector((center.x,center.y,0))+axis*a+cross*b for a,b in [(-width/2,-width/2),(width/2,-width/2),(width/2,width/2),(-width/2,width/2)]];v=[Vector((p.x,p.y,z))for z in [low,high]for p in xy];return mesh(name,v,[(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)],35)
 front_post=R+Vector((-3.4,0,0));post('Market well / rear timber post',L,floor,L.z-3.2,2.2);post('Market well / front timber post',front_post,floor,R.z-3.2,2.2)
 # A narrow ridge beam joins the two anchored roof posts beneath the shells.
 beam=[p+cross*a+Vector((0,0,z))for z in [-5.0,-2.4]for p,a in [(L,-1.8),(front_post,-1.8),(front_post,1.8),(L,1.8)]]
 mesh('Market well / ridge support beam',beam,[(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)],35)
 # The separate small pail is traced from its bright rim and compact body,
 # excluding the broad cast shadow. Hidden cavity and backside are inferred.
 count=20;v=[]
 for radius,z in [(2.7,floor),(3.5,floor+6),(2.8,floor+6),(2.1,floor+.8)]:
  for i in range(count):
   a=2*math.pi*i/count;v.append((246+radius*math.cos(a),-2264.6+radius*math.sin(a),z))
 f=[tuple(range(count-1,-1,-1)),tuple(range(3*count,4*count))]
 for band in range(3):
  for i in range(count):j=(i+1)%count;f.append((band*count+i,band*count+j,(band+1)*count+j,(band+1)*count+i))
 bucket=mesh('Market well / small open tapered pail',v,f,35);bucket['source_role']='Inferred small pail from visible rim and dark compact body; separate traced source domain, no surrounding shadow.'
 bpy.context.view_layer.update();tree,owners,_=_tree(own+context);alphas={o:np.zeros(domain.shape,dtype=np.uint8)for o in own};missing=[];foreign={}
 for yy,xx in np.argwhere(domain>0):
  x=int(xx)+BOX[0];y=int(yy)+BOX[1];p,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/S,0))+RAY*5000,-RAY)
  if p is None:missing.append([x,y])
  elif owners[i]in alphas:alphas[owners[i]][yy,xx]=domain[yy,xx]
  else:foreign.setdefault(owners[i].name,[]).append([x,y])
 ownership={}
 for index,(o,a)in enumerate(alphas.items()):
  path=D/f'owned-{index}.png';atlas=art.crop(BOX);atlas.putalpha(Image.fromarray(a));atlas.save(path);image=bpy.data.images.load(str(path),check_existing=False);image.pack();o.data.materials[0]=material(o.name+' observed',image);ownership[o.name]=dict(accepted_pixels=int((a>0).sum()),atlas=str(path),sha256=sha(path))
 assert all(sig(o)==before[o.name]for o in context)
 scene.render.engine='CYCLES';scene.cycles.samples=12;scene.render.film_transparent=True;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Well diagnostic world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.12,.12,.12,1)
 bpy.context.preferences.filepaths.save_version=0
 for other in list(bpy.data.scenes):
  if other!=scene:bpy.data.scenes.remove(other)
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'));assert sha(SOURCE)==source_hash
 report=dict(source=str(SOURCE),source_sha256=source_hash,model_sha256=sha(D/'model.blend'),asset=ASSET,context_exact=before,native_box=BOX,native_masks=[49,50],bucket_inferred_source_polygon=bucket_polygon,domain_pixels=int((domain>0).sum()),ownership=ownership,missing=missing,foreign=foreign,floor_z=floor,inferred=['Closed stone ring rear and inner wall depth','Two full timber posts and connecting ridge support','Roof underside thickness','Small pail hidden backside, taper and cavity depth'],limitations=['Bucket source domain is explicitly traced and inferred separately from native masks49/50; surrounding ground shadow is excluded.'],status='Private geometry candidate; all-angle and source/contact review pending')
 (D/'construction.json').write_text(json.dumps(report,indent=2)+'\n');print('OWN',sum(x['accepted_pixels']for x in ownership.values()),'DOMAIN',report['domain_pixels'],'MISS',len(missing),'FOREIGN',sum(map(len,foreign.values())))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
