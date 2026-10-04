"""Build and review a solid rotating mission sign from native front/back poses."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
from fit_native_sign import OUT,SIN,COS,geometry,FACES,projected


def material(name,color=None,path=None):
 m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;n.clear();out=n.new('ShaderNodeOutputMaterial');shader=n.new('ShaderNodeBsdfPrincipled');shader.inputs['Roughness'].default_value=1;m.node_tree.links.new(shader.outputs[0],out.inputs[0])
 if path:
  tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path),check_existing=False);tex.image.pack();tex.interpolation='Closest';m.node_tree.links.new(tex.outputs['Color'],shader.inputs['Base Color']);m.node_tree.links.new(tex.outputs['Color'],shader.inputs['Emission Color']);shader.inputs['Emission Strength'].default_value=.7
 else:shader.inputs['Base Color'].default_value=(*color,1)
 return m


def face_image(vertices,face,p,frame,source,dest,is_board):
 # Bilinear face coordinates preserve a single coherent timber surface.
 size=128;v=vertices[list(face)];uu,vv=np.meshgrid((np.arange(size)+.5)/size,(np.arange(size)+.5)/size)
 pts=((1-uu)*(1-vv))[:,:,None]*v[0]+(uu*(1-vv))[:,:,None]*v[1]+(uu*vv)[:,:,None]*v[2]+((1-uu)*vv)[:,:,None]*v[3]
 coords=projected(pts.reshape(-1,3),p,frame)-[32,55];f=source['frames'][frame];sprite=np.asarray(Image.open(f['image']).convert('RGBA'));xy=np.floor(coords-np.asarray(f['offset'])).astype(int);valid=(xy[:,0]>=0)&(xy[:,0]<sprite.shape[1])&(xy[:,1]>=0)&(xy[:,1]<sprite.shape[0]);rgb=np.full((size*size,3),115,dtype=np.uint8);sample=np.zeros((size*size,4),dtype=np.uint8);sample[valid]=sprite[xy[valid,1],xy[valid,0]];known=valid&(sample[:,3]>127)
 # The physical rear post is in front of the board in pose13; its source pixels
 # cannot be reused as board appearance. Hidden timber remains neutral.
 if is_board and frame==13:known &= np.abs(coords[:,0]-p[8])>p[4]/2+.7
 if not is_board and frame!=13:known &= (pts[:,:,2].ravel()<p[2]-1)|(pts[:,:,2].ravel()>p[2]+p[1]+1)
 rgb[known]=sample[known,:3];Image.fromarray(np.flipud(rgb.reshape(size,size,3))).save(dest)
 return dict(frame=frame,known_texels=int(known.sum()),unknown_texels=int((~known).sum()),image_sha256=sha(dest),source_sha256=f['image_sha256'],scope='Exact nearest native RGB on visible face hypothesis; hidden or outside source remains neutral gray')


def main():
 base=OUT/'state-sign-candidate';fit_path=base/'fit-v6/fit.json';fit=json.loads(fit_path.read_text());p=list(fit['parameters'].values());dst=base/'candidate-v2';dst.mkdir(exist_ok=False)
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Croisement02 mission sign candidate';scene.render.engine='CYCLES';scene.cycles.samples=24;scene.cycles.transparent_max_bounces=64;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.fps=25
 world=bpy.data.worlds.new('Neutral inspection');world.use_nodes=True;world.node_tree.nodes['Background'].inputs[0].default_value=(.4,.4,.4,1);world.node_tree.nodes['Background'].inputs[1].default_value=.8;scene.world=world
 root=bpy.data.objects.new('Rotating sign pivot',None);scene.collection.objects.link(root);root['mission_profile']='Panneau';root['mission_visibility']='S03_FoB_MP';root['source_node']='mission-panneau';root['native_action_aliases']=[0,210,211]
 row=next(r for r in fit['profile']['rows']if r['action_id']==0);objects=[];materials=[];face_receipts=[]
 for part,vertices in zip(['board','post'],geometry(p)):
  mesh=bpy.data.meshes.new(part);mesh.from_pydata(vertices.tolist(),[],FACES);mesh.update();o=bpy.data.objects.new('Panneau '+part,mesh);scene.collection.objects.link(o);o.parent=root;o['asset_group']='croisement02-mission-rotating-sign';o['source_node']='mission-panneau';o['mission_profile']='Panneau';o['part_name']=part
  uv=mesh.uv_layers.new(name='Native face projection')
  for i,poly in enumerate(mesh.polygons):
   # Faces2/4 are local negative/positiveY. Side poses support board thickness.
   frame={2:29,4:13,3:5,5:21,0:29,1:29}[i];path=dst/f'{part}-face-{i}.png';receipt=face_image(vertices,FACES[i],p,frame,row,path,part=='board');face_receipts.append(dict(part=part,face=i,**receipt));mat=material(f'{part} source pose{frame} face{i}',path=path);mat['appearance_provenance']=receipt['scope'];mesh.materials.append(mat);materials.append(mat);poly.material_index=i
   for loop,coord in zip(poly.loop_indices,[(0,0),(1,0),(1,1),(0,1)]):uv.data[loop].uv=coord
  bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();objects.append(o)
 for i in range(33):
  root.rotation_euler.z=math.radians(p[7]-i*11.25);root.keyframe_insert(data_path='rotation_euler',index=2,frame=1+2*i)
 if root.animation_data and root.animation_data.action:
  root.animation_data.action.name='Panneau native rotation 64ticks'
  for layer in root.animation_data.action.layers:
   for strip in layer.strips:
    for bag in strip.channelbags:
     for curve in bag.fcurves:
      for key in curve.keyframe_points:key.interpolation='CONSTANT'
 scene.frame_start=1;scene.frame_end=64;scene.frame_set(1)
 sun=bpy.data.lights.new('Inspection sun','SUN');sun.energy=2;so=bpy.data.objects.new(sun.name,sun);scene.collection.objects.link(so);so.rotation_euler=(.5,-.4,-.5)
 bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));model_hash=sha(dst/'model.blend')
 # Inspect the saved file, preserving the actual animation/material bytes.
 bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;objects=[scene.objects['Panneau '+s]for s in ['board','post']];target=Vector((0,0,24));data=bpy.data.cameras.new('Review camera');data.type='ORTHO';data.ortho_scale=68;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
 solid=material('Review neutral solid',color=(.5,.5,.5));saved=[list(o.data.materials)for o in objects];views=dst/'views';views.mkdir();receipts=[]
 for mode in ['textured','solid']:
  if mode=='solid':
   for o in objects:
    for i in range(len(o.data.materials)):o.data.materials[i]=solid
  sheet=Image.new('RGB',(1536,768),(40,40,40))
  for i in range(8):
   az=math.radians(i*45);direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN));camera.location=target+direction*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(views/f'view-{i}-{mode}.png');bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');tile=Image.new('RGB',im.size,(40,40,40));tile.paste(im,mask=im.getchannel('A'));sheet.paste(tile,((i%4)*384,(i//4)*384))
  sheet.save(dst/f'{mode}.png')
 write_json(dst/'evidence.json',dict(status='Private geometry and material candidate; independent all-eight review pending',model_sha256=model_hash,fit_sha256=sha(fit_path),source_silhouette_fit_iou=fit['aggregate_iou'],face_appearance=face_receipts,instances=fit['instances'],timing=fit['timing'],geometry=dict(meshes=2,closed_solids=True,board_thickness=p[5],inferred='Rectangular timber profiles, unseen thickness and supporting post depth'),limitations=['Neutral hidden face texels require appearance completion.','Source silhouette fit uses a provisional body/shadow split; source-edge discrepancies remain reviewable.','Placement on five terrain receivers and rendered animation validation pending.','No canonical catalog or approval changed.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
