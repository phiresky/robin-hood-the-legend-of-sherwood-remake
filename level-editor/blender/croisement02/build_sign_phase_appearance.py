"""Preserve native per-pose timber shading on the solid rotating sign."""
import json,sys,math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from fit_native_sign import geometry,FACES,projected,SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release
from build_sign_shadow_proof import constant_action

def normals(vertices):
 result=[];center=vertices.mean(0)
 for face in FACES:
  v=vertices[list(face)];n=np.cross(v[1]-v[0],v[2]-v[0]);n/=np.linalg.norm(n)
  if np.dot(n,v[0]-center)<0:n=-n
  result.append((n,v[0]))
 return result

def blocked(points,direction,planes):
 # Intersect a ray toward the camera with the other complete convex timber.
 lo=np.full(len(points),1e-3);hi=np.full(len(points),np.inf);possible=np.ones(len(points),bool)
 for normal,origin in planes:
  numerator=(origin-points)@normal;denom=float(direction@normal)
  if abs(denom)<1e-8:possible&=numerator>=0
  elif denom>0:hi=np.minimum(hi,numerator/denom)
  else:lo=np.maximum(lo,numerator/denom)
 return possible&(hi>=lo)&(hi>1e-3)

def main():
 base=OUT/'state-sign-candidate';src=base/'shadow-proof-v2/model.blend';source_hash=sha(src);fit_path=base/'fit-v6/fit.json';fit=json.loads(fit_path.read_text());p=list(fit['parameters'].values());row=next(r for r in fit['profile']['rows']if r['action_id']==0);dst=base/'phase-appearance-v1';dst.mkdir(exist_ok=False);vertices=geometry(p);planes=[normals(v)for v in vertices];receipts=[];images={};size=128;uu,vv=np.meshgrid((np.arange(size)+.5)/size,(np.arange(size)+.5)/size)
 for phase in range(32):
  f=row['frames'][phase];assert sha(Path(f['image']))==f['image_sha256'];sprite=np.asarray(Image.open(f['image']).convert('RGBA'));angle=math.radians(p[7]-phase*11.25);direction=np.array([-math.sin(angle)*COS,-math.cos(angle)*COS,SIN])
  for part_index,part in enumerate(['board','post']):
   for face_index,face in enumerate(FACES):
    v=vertices[part_index][list(face)];pts=(((1-uu)*(1-vv))[:,:,None]*v[0]+(uu*(1-vv))[:,:,None]*v[1]+(uu*vv)[:,:,None]*v[2]+((1-uu)*vv)[:,:,None]*v[3]).reshape(-1,3);screen=projected(pts,p,phase)-[32,55];xy=np.floor(screen-np.asarray(f['offset'])).astype(int);valid=(xy[:,0]>=0)&(xy[:,0]<sprite.shape[1])&(xy[:,1]>=0)&(xy[:,1]<sprite.shape[0]);sample=np.zeros((size*size,4),np.uint8);sample[valid]=sprite[xy[valid,1],xy[valid,0]];known=valid&(sample[:,3]>127)&(float(planes[part_index][face_index][0]@direction)>.05)&~blocked(pts,direction,planes[1-part_index]);donor=np.asarray(Image.open(base/f'native-fill-v1/{part}-face-{face_index}.png'));rgb=np.flipud(donor).reshape(-1,3).copy();rgb[known]=sample[known,:3];assert np.array_equal(rgb[known],sample[known,:3]);path=dst/f'pose-{phase:02}-{part}-face-{face_index}.png';Image.fromarray(np.flipud(rgb.reshape(size,size,3))).save(path);mask=path.with_name(path.stem+'-ownership.png');Image.fromarray(np.flipud(known.reshape(size,size).astype('uint8')*255)).save(mask);images[(phase,part,face_index)]=path;receipts.append(dict(phase=phase,part=part,face=face_index,known_texels=int(known.sum()),inferred_texels=int((~known).sum()),image_sha256=sha(path),ownership_sha256=sha(mask),source_sha256=f['image_sha256'],known_rgb_changed=0))
 bpy.ops.wm.open_mainfile(filepath=str(src));scene=bpy.context.scene;root=scene.objects['Rotating sign pivot'];oldparts=[scene.objects['Panneau '+s]for s in ['board','post']];before={o.name:dict(vertices=[list(v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],uvs=[list(l.uv)for l in o.data.uv_layers.active.data])for o in oldparts};parts=[]
 for phase in range(32):
  for part,old in zip(['board','post'],oldparts):
   obj=old.copy();obj.name=f'Panneau {part} pose {phase:02}';scene.collection.objects.link(obj);obj['native_body_frame']=phase;obj['native_phase_part']=part;parts.append(obj)
   for i,slot in enumerate(obj.material_slots):
    mat=bpy.data.materials.new(f'Native sign pose {phase:02} {part} face {i}');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(images[(phase,part,i)]),check_existing=False);tex.image.pack();tex.interpolation='Closest';emit=n.new('ShaderNodeEmission');out=n.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(tex.outputs['Color'],emit.inputs[0]);mat.node_tree.links.new(emit.outputs[0],out.inputs[0]);mat['appearance_provenance']='Exact pose-specific visible native RGB; hidden texels use own completed timber';slot.link='OBJECT';slot.material=mat
   for frame,value in sorted({1:1 if phase==0 else 0,1+phase*2:1,3+phase*2:0,65:1 if phase==0 else 0}.items()):obj.scale=(value,value,value);obj.keyframe_insert(data_path='scale',frame=frame)
   constant_action(obj)
 for obj in oldparts:bpy.data.objects.remove(obj,do_unlink=True)
 scene.frame_set(1);bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;parts=[o for o in scene.objects if 'native_body_frame'in o];assert len(parts)==64
 for obj in parts:
  prior=before['Panneau '+obj['native_phase_part']];current=dict(vertices=[list(v.co)for v in obj.data.vertices],faces=[list(p.vertices)for p in obj.data.polygons],uvs=[list(l.uv)for l in obj.data.uv_layers.active.data]);assert current==prior
 data=bpy.data.cameras.new('Native phase appearance camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=64;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;target=Vector((0,19/SIN,0));camera.location=target+Vector((0,-COS,SIN))*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=256;scene.render.resolution_y=288;scene.render.resolution_percentage=100;scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128;sheet=Image.new('RGB',(8*256,4*312),(80,80,80));comparisons=Image.new('RGB',(8*256,4*288),(80,80,80));gif=[];checks=[]
 for phase in range(33):
  scene.frame_set(1+phase*2);visible=[o for o in parts if o.scale.x>.5];assert len(visible)==2 and all(o['native_body_frame']==phase%32 for o in visible)
  if phase==32:continue
  path=dst/f'pose-{phase:02}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);render=Image.open(path).convert('RGBA');canvas=Image.new('RGB',render.size,(80,80,80));canvas.paste(render,mask=render.getchannel('A'));sheet.paste(canvas,((phase%8)*256,(phase//8)*312));ImageDraw.Draw(sheet).text(((phase%8)*256+3,(phase//8)*312+290),str(phase),fill='white');gif.append(canvas)
  f=row['frames'][phase];native=Image.new('RGBA',(64,72));native.alpha_composite(Image.open(f['image']).convert('RGBA'),(32+int(f['offset'][0]),55+int(f['offset'][1])));pair=Image.new('RGB',(512,288),(80,80,80));native=native.resize((256,288),Image.Resampling.NEAREST);pair.paste(native,mask=native.getchannel('A'));pair.paste(canvas,(256,0));pair.save(dst/f'comparison-{phase:02}.png');comparisons.paste(pair.resize((256,144),Image.Resampling.NEAREST),((phase%8)*256,(phase//8)*288))
 sheet.save(dst/'actual-32-poses.png');comparisons.save(dst/'source-comparison.png');gif[0].save(dst/'native-motion.gif',save_all=True,append_images=gif[1:],duration=80,loop=0);assert sha(src)==source_hash;write_json(dst/'evidence.json',dict(status='Private per-pose appearance correction; actual source/oblique review pending',model_sha256=digest,source_model_sha256=source_hash,fit_sha256=sha(fit_path),faces=receipts,known_rgb_changed=0,body_geometry_uv_unchanged=True,visible_body_parts_per_pose=2,cycle_wrap_checked=True,limitations=['Pose-specific source RGB uses fitted physical surface visibility; source contour disagreement remains separate.','Hidden texels retain own inferred timber appearance, not new observed evidence.','Representation uses64 phase-specific body instances with constant scale visibility; only2 are visible per pose.','Physical scene and export/editor animation remain unverified.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
