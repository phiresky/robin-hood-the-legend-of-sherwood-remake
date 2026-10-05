"""Build source-preserving net03 endpoints with a separate inferred hidden support."""
import json,math,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw,ImageFilter
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from scenery_geometry import Mesh
from log_trap_state_candidate import point,material
from evidence_io import sha,write_json
from render_slots import acquire,release
from review_bank_candidate import camera
from restore_ground75_source import geometry
DEST=OUT/'restart3-net03/endpoints-v4'
SHIFT=50


def mesh_object(scene,name,vertices,faces):
 data=bpy.data.meshes.new(name);data.from_pydata(vertices,[],faces);data.update();bm=bmesh.new();bm.from_mesh(data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(data);bm.free();data.update();o=bpy.data.objects.new(name,data);scene.collection.objects.link(o);return o


def volume(obj):
 bm=bmesh.new();bm.from_mesh(obj.data);closed=all(e.is_manifold for e in bm.edges);v=bm.calc_volume(signed=True);bm.free()
 if not closed or v<=0:raise ValueError('Nonclosed volume '+obj.name)
 return v


def project(obj,row,known,gray):
 obj.data.materials.clear();obj.data.materials.append(known);obj.data.materials.append(gray);uv=obj.data.uv_layers.get('Native target projection') or obj.data.uv_layers.new(name='Native target projection');x,y,w,h=row['bbox']
 for f in obj.data.polygons:
  f.material_index=0 if f.normal.dot(RAY)>.05 else 1
  for li in f.loop_indices:
   p=obj.data.vertices[obj.data.loops[li].vertex_index].co;uv.data[li].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)


def sheet(paths,dest):
 out=Image.new('RGB',(1536,768),'#333')
 for i,p in enumerate(paths):
  im=Image.open(p).convert('RGBA');bg=Image.new('RGBA',im.size,'#333');bg.alpha_composite(im);out.paste(bg.convert('RGB').resize((384,384)),(i%4*384,i//4*384))
 out.save(dest)


def main():
 if DEST.exists():raise FileExistsError(DEST)
 fit=OUT/'net-endpoint-volume-fit-v2/manifest.json';rows=[r for r in json.loads(fit.read_text())['records']if r['family']=='piege03'];survey=OUT/'restart3-net03/support-survey-v1';tree=json.loads((survey/'tree-39-wood.json').read_text());worker=Path(tree['worker']);assert sha(worker)==tree['model_sha256']
 acquire()
 try:
  DEST.mkdir(parents=True)
  bpy.ops.wm.open_mainfile(filepath=str(worker));bpy.context.view_layer.update();tree_names=[o.name for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-39'];tree_signatures={n:geometry(bpy.data.objects[n]) for n in tree_names}
  bvh=BVHTree.FromPolygons(tree['vertices'],tree['triangles'],all_triangles=True);surface,normal,face,distance=bvh.find_nearest(Vector((1688,-1137.58563666012,210)));root=surface-normal*1.5
  nety=-629/SIN-SHIFT*COS;branch_points=[root,Vector((1675,nety,212)),Vector((1655,nety,215)),Vector((1627,nety,212))];radii=[4.5,3.5,2.4,1.2]
  for row in rows:
   suffix=row['variant'];folder=DEST/suffix;folder.mkdir();source=Path(row['source']);assert sha(source)==row['source_sha256'];raw=np.array(Image.open(source).convert('RGBA'));x,y,w,h=row['bbox'];cx,bottom,height,tilt,*bag_radii=row['parameters'];lift=(629-y-bottom)/COS
   bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Net03 '+suffix;scene.world=bpy.data.worlds.new('Neutral world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.2,.2,.2,1);sun=bpy.data.objects.new('Review sun',bpy.data.lights.new('Review sun','SUN'));scene.collection.objects.link(sun);sun.data.energy=2;sun.rotation_euler=(.6,-.5,-.4)
   gray=bpy.data.materials.new('Unknown net and inferred support');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.17,.17,.17,1);gray.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1
   polygons={'bag':row['outline_crop_coordinates'],'wood':[(0,46),(23,39),(31,42),(29,60),(9,68),(0,63)],'bag-cord':[(40,0),(45,0),(46,20),(39,20)],'wood-cord':[(23,0),(27,0),(28,45),(23,46)]}
   masks={}
   for name,poly in polygons.items():
    mask=Image.new('L',(w,h));ImageDraw.Draw(mask).polygon(poly,fill=255)
    if name=='bag':mask=mask.filter(ImageFilter.MaxFilter(5))
    masks[name]=np.array(mask)>0
   # The visible bag occludes the right counterweight in the occupied state.
   if suffix=='i':masks['wood'][:,18:]=False
   # Trace the connected bag silhouette; detached sprite fragments are not solid cloth.
   from collections import deque
   admitted=masks['bag'] & (raw[:,:,3]>0)
   if suffix=='e':
    admitted=raw[:,:,3]>0;admitted[:,:26]=False
   admitted[:18]=False;remaining=admitted.copy();components=[]
   for yy,xx in np.argwhere(admitted):
    if not remaining[yy,xx]:continue
    queue=deque([(int(yy),int(xx))]);remaining[yy,xx]=False;component=[]
    while queue:
     cy,cx0=queue.popleft();component.append((cy,cx0))
     for dy,dx in [(-1,0),(1,0),(0,-1),(0,1)]:
      ny,nx=cy+dy,cx0+dx
      if 0<=ny<h and 0<=nx<w and remaining[ny,nx]:remaining[ny,nx]=False;queue.append((ny,nx))
    components.append(component)
   body=np.zeros((h,w),bool)
   for yy,xx in max(components,key=len):body[yy,xx]=True
   masks['bag']=body;materials={};native_guard=[]
   for name,mask in masks.items():
    pixels=raw.copy();pixels[~mask,3]=0;path=folder/(name+'-observed.png');Image.fromarray(pixels).save(path);assert np.array_equal(pixels[mask],raw[mask]);materials[name]=material(path);native_guard.append(dict(component=name,observed_pixels=int((mask&(raw[:,:,3]>0)).sum()),source_rgba_exact=True,image_sha256=sha(path)))
   vs=[];fs=[];n=48;source_rings=[]
   for sy in np.flatnonzero(body.any(axis=1)):
    xs=np.flatnonzero(body[sy]);source_rings.append((float(sy)+.5,float(xs.min()),float(xs.max()+1)))
   source_rings=[(source_rings[0][0]-.75,sum(source_rings[0][1:])/2-.1,sum(source_rings[0][1:])/2+.1),*source_rings,(source_rings[-1][0]+.75,sum(source_rings[-1][1:])/2-.1,sum(source_rings[-1][1:])/2+.1)]
   for sy,left,right in reversed(source_rings):
    center_x=(left+right)/2;r=(right-left)/2;base_z=(629-y-sy)/COS
    for j in range(n):
     a=j*math.tau/n;vs.append(point(x+center_x+r*math.cos(a),y+sy,base_z)+Vector(RAY)*(SHIFT+r*row['depth_to_horizontal_radius']*math.sin(a)))
   for k in range(len(source_rings)-1):
    for j in range(n):a=k*n+j;b=k*n+(j+1)%n;fs.append((a,b,b+n,a+n))
   fs.extend([tuple(range(n-1,-1,-1)),tuple((len(source_rings)-1)*n+j for j in range(n))]);bag=mesh_object(scene,'Net03 '+suffix+' bag',vs,fs)
   wood_z=(629-586)/COS+SHIFT*SIN;m=Mesh();m.tube(point(x+5,y+56,wood_z),point(x+26,y+48,wood_z),10.2,n=24);wood=mesh_object(scene,'Net03 counterweight',m.vertices,m.faces)
   for v in wood.data.vertices:
    sy=-v.co.y*SIN-v.co.z*COS-y;v.co.z-=(1.8+.12*(sy-54))/COS
   wood.data.update();project(wood,row,materials['wood'],gray)
   before=volume(bag);bpy.context.view_layer.objects.active=bag;mod=bag.modifiers.new('Local cloth around counterweight','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=wood;bpy.ops.object.modifier_apply(modifier=mod.name);after=volume(bag)
   if after<before*.8:raise ValueError('Excess inferred cloth carving')
   # Preserve the larger observed openings between gathered net straps as real air.
   openings=np.zeros((h,w),bool)
   for sy in range(18,min(43,h)):
    xs=np.flatnonzero(body[sy])
    if len(xs):openings[sy,xs.min():xs.max()+1]=raw[sy,xs.min():xs.max()+1,3]==0
   unseen=openings.copy();hole_groups=[]
   for yy,xx in np.argwhere(openings):
    if not unseen[yy,xx]:continue
    queue=deque([(int(yy),int(xx))]);unseen[yy,xx]=False;group=[]
    while queue:
     cy,cx0=queue.popleft();group.append((cy,cx0))
     for dy,dx in [(-1,0),(1,0),(0,-1),(0,1)]:
      ny,nx=cy+dy,cx0+dx
      if 0<=ny<h and 0<=nx<w and unseen[ny,nx]:unseen[ny,nx]=False;queue.append((ny,nx))
    if len(group)>=6:hole_groups.append(group)
   for hi,group in enumerate(hole_groups):
    cells=set(group);verts=[];faces=[];lookup={}
    def vertex(px,py,side):
     key=(px,py,side)
     if key not in lookup:lookup[key]=len(verts);verts.append(point(x+px,y+py,0)+Vector(RAY)*(-300 if side==0 else 500))
     return lookup[key]
    for cy,cx0 in group:
     corners=[(cx0,cy),(cx0+1,cy),(cx0+1,cy+1),(cx0,cy+1)]
     faces.append(tuple(vertex(px,py,0)for px,py in reversed(corners)));faces.append(tuple(vertex(px,py,1)for px,py in corners))
     for edge,(dy,dx) in enumerate([(-1,0),(0,1),(1,0),(0,-1)]):
      if (cy+dy,cx0+dx) in cells:continue
      a=corners[edge];b=corners[(edge+1)%4];faces.append((vertex(*a,0),vertex(*b,0),vertex(*b,1),vertex(*a,1)))
    cutter=mesh_object(scene,'Source strap opening cutter '+str(hi),verts,faces);volume(cutter);bpy.context.view_layer.objects.active=bag;mod=bag.modifiers.new('Observed strap opening','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(cutter,do_unlink=True)
   volume(bag)
   project(bag,row,materials['bag'],gray)
   support_vs=[];support_fs=[];rings=12
   for center,r in zip(branch_points,radii):
    for j in range(rings):a=j*math.tau/rings;support_vs.append(center+Vector((0,math.cos(a)*r,math.sin(a)*r)))
   for k in range(3):
    for j in range(rings):a=k*rings+j;b=k*rings+(j+1)%rings;support_fs.append((a,b,b+rings,a+rings))
   support_fs.extend([tuple(range(rings-1,-1,-1)),tuple(3*rings+j for j in range(rings))]);support=mesh_object(scene,'Net03 inferred hidden support branch',support_vs,support_fs);support.data.materials.append(gray);support['inference']='Separate net-support component. Existing approved tree geometry unchanged.'
   attachments=[]
   cord_pixels=[]
   for sy in range(18):
    cord_pixels.extend((sy+.5,float(sx)+38.5)for sx in np.flatnonzero(raw[sy,38:48,3]>0))
   native_slope,native_intercept=np.polyfit(np.array(cord_pixels)[:,0],np.array(cord_pixels)[:,1],1)
   bag_base=sum(vs[-n:],Vector())/n;cordx=bag_base.x
   for _ in range(12):
    z=212+(cordx-1627)/28*3;source_y=-nety*SIN-z*COS-y;cordx=x+native_slope*source_y+native_intercept
   for name,cordx,base,radius in [('bag-cord',float(cordx),bag_base,1.5),('wood-cord',1629,Vector((1629,nety,(-nety*SIN-575)/COS)),1.0)]:
    z=212+(cordx-1627)/28*3;end=Vector((cordx,nety,z));m=Mesh();m.tube(base-Vector((0,0,.3)),end,radius,n=12);o=mesh_object(scene,'Net03 '+name,m.vertices,m.faces);project(o,row,materials[name],gray);attachments.append(dict(cord=name,lower=list(base),upper=list(end),radius=radius,support_radius_at_attachment=1.2+(cordx-1627)/28*1.2,native_lean_fit=[float(native_slope),float(native_intercept)]if name=='bag-cord'else None))
   objects=[o for o in scene.objects if o.type=='MESH'];guards=[dict(name=o.name,closed_volume=volume(o),geometry_uv_signature=geometry(o))for o in objects]
   for o in objects:o['asset_group']='croisement02-net-piege03';o['state_variant']=suffix+' final phase0';o['source_patch']=row['patch_id']
   center=Vector((1652,nety,135));camera(scene,center,Vector(RAY),384,384,245);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True);model_hash=sha(folder/'model.blend')
   bpy.ops.wm.open_mainfile(filepath=str(folder/'model.blend'));scene=bpy.context.scene
   for g in guards:
    o=bpy.data.objects[g['name']]
    if geometry(o)!=g['geometry_uv_signature']:raise ValueError('Reopened geometry changed')
    volume(o)
   actual=[];solid=[]
   for i in range(8):
    angle=i*math.pi/4;direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,center,direction,384,384,245)
    for mode,paths in [('actual',actual),('solid',solid)]:
     scene.view_layers[0].material_override=bpy.data.materials['Unknown net and inferred support']if mode=='solid'else None;path=folder/f'{i:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);paths.append(path)
   sheet(actual,folder/'actual8.png');sheet(solid,folder/'solid8.png');scene.view_layers[0].material_override=None
   camera(scene,point(x+w/2,y+h/2,0),Vector(RAY),w*5,h*5,w);scene.render.filepath=str(folder/'native.png');bpy.ops.render.render(write_still=True)
   compare=Image.new('RGB',(w*10,h*5),'#333')
   for i,path in enumerate([source,folder/'native.png']):
    im=Image.open(path).convert('RGBA');bg=Image.new('RGBA',im.size,'#333');bg.alpha_composite(im);compare.paste(bg.resize((w*5,h*5),Image.Resampling.NEAREST).convert('RGB'),(i*w*5,0))
   compare.save(folder/'source-native.png')
   with bpy.data.libraries.load(str(worker),link=False) as (src,dst):dst.objects=list(tree_names)
   for o in dst.objects:scene.collection.objects.link(o)
   for o in dst.objects:
    parent=o.parent
    while parent is not None:
     if parent.name not in scene.objects:scene.collection.objects.link(parent)
     parent=parent.parent
   bpy.context.view_layer.update()
   for o in dst.objects:
    if geometry(o)!=tree_signatures[o.name]:raise ValueError('Tree import transform changed')
    o.hide_render=False
   for i,angle in enumerate([0,math.pi/4,-math.pi/3]):
    direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,Vector((1665,nety,155)),direction,640,640,370);scene.render.filepath=str(folder/f'contact-{i}.png');bpy.ops.render.render(write_still=True)
   for o in dst.objects:
    if o.get('projection_component')=='crown':o.hide_render=True
   camera(scene,Vector((1655,nety,145)),Vector((COS*.7071,-COS*.7071,SIN)),640,640,300);scene.render.filepath=str(folder/'contact-wood-only.png');bpy.ops.render.render(write_still=True)
   write_json(folder/'manifest.json',dict(status='private endpoint; self-review pending',model_sha256=model_hash,source_sha256=sha(source),fit_manifest_sha256=sha(fit),objects=guards,native_rgba_guards=native_guard,cloth_removed_fraction=(before-after)/before,source_camera_first=True,ground_reference_source_y=629,source_preserving_depth_shift=SHIFT,bag_construction='Connected native silhouette cross sections with conservatively inferred depth; no generic profile silhouette',source_rings=source_rings,source_air_openings=[dict(pixels=len(g),coordinates=g)for g in hole_groups],attachments=attachments,tree_worker=str(worker),tree_sha256=tree['model_sha256'],inferred_support=dict(points=[list(p)for p in branch_points],radii=radii,existing_wood_surface=list(surface),surface_normal=list(normal),root_penetration=1.5),limitations=['Hidden branch extension is an explicit net-support inference under canopy, not a mutation or approval of tree39.','Only final phase0 e/i endpoints are modeled; animation, initial rigging, captured actors and effects remain separate.','Gray surfaces have no inferred/generated appearance yet.','Manual source outlines carry approximately two native pixels of uncertainty.'],renders={p.name:sha(p)for p in folder.glob('*.png')},user_approval=None))
  if sha(worker)!=tree['model_sha256']:raise ValueError('Support tree changed')
 finally:release()
if __name__=='__main__':main()
