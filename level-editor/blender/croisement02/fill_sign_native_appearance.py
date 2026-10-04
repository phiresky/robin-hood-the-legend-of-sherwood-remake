"""Complete sign-only hidden texels from its own observed reverse timber."""
import json,sys,hashlib
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from fit_native_sign import geometry,FACES,SIN,COS
from build_sign_candidate import face_image
from evidence_io import sha,write_json
from render_slots import acquire,release


def fingerprint(obj):
 return hashlib.sha256(repr(([(tuple(v.co))for v in obj.data.vertices],[tuple(p.vertices)for p in obj.data.polygons],[tuple(x.uv)for x in obj.data.uv_layers.active.data],list(map(list,obj.matrix_world)))).encode()).hexdigest()


def main():
 base=OUT/'state-sign-candidate';candidate=base/'candidate-v2';dst=base/'native-fill-v1';dst.mkdir(exist_ok=False);fit=json.loads((base/'fit-v6/fit.json').read_text());p=list(fit['parameters'].values());row=next(r for r in fit['profile']['rows']if r['action_id']==0);source_model=candidate/'model.blend';source_hash=sha(source_model);generated={}
 for part,vertices in zip(['board','post'],geometry(p)):
  for i,face in enumerate(FACES):
   frame={2:29,4:13,3:5,5:21,0:29,1:29}[i];path=dst/f'{part}-face-{i}-known.png';face_image(vertices,face,p,frame,row,path,part=='board');old=np.asarray(Image.open(candidate/f'{part}-face-{i}.png'));regenerated=np.asarray(Image.open(path));assert np.array_equal(old,regenerated);known=np.asarray(Image.open(path.with_name(path.stem+'-ownership.png')))>0;generated[(part,i)]=(old,known)
 # Each hidden board face uses the observed reverse plank face. Each hidden
 # post face uses the observed rear post, keeping its vertical grain direction.
 receipts=[]
 for part in ['board','post']:
  donor,donor_known=generated[(part,4)];assert donor_known.any();nearest=distance_transform_edt(~donor_known,return_distances=False,return_indices=True);completed_donor=donor[nearest[0],nearest[1]]
  for i in range(6):
   old,known=generated[(part,i)];filled=old.copy();filled[~known]=completed_donor[~known];assert np.array_equal(old[known],filled[known]);path=dst/f'{part}-face-{i}.png';Image.fromarray(filled).save(path);receipts.append(dict(part=part,face=i,known_texels=int(known.sum()),inferred_filled_texels=int((~known).sum()),known_rgb_unchanged=True,source_image_sha256=sha(candidate/f'{part}-face-{i}.png'),output_sha256=sha(path),donor='Own observed reverse '+part+' face from native pose13',scope='Inferred hidden appearance, not observed front evidence'))
 bpy.ops.wm.open_mainfile(filepath=str(source_model));scene=bpy.context.scene;parts=[scene.objects['Panneau '+s]for s in ['board','post']];before={o.name:fingerprint(o)for o in parts}
 for obj,part in zip(parts,['board','post']):
  for i,mat in enumerate(obj.data.materials):
   tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');tex.image=bpy.data.images.load(str(dst/f'{part}-face-{i}.png'),check_existing=False);tex.image.pack();mat['hidden_appearance']='Inferred using own native reverse timber; observed texels preserved exactly'
 assert before=={o.name:fingerprint(o)for o in parts};bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;parts=[scene.objects['Panneau '+s]for s in ['board','post']];assert before=={o.name:fingerprint(o)for o in parts};data=bpy.data.cameras.new('Actual filled sign review');data.type='ORTHO';data.ortho_scale=68;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;target=Vector((0,0,24));views=dst/'views';views.mkdir();sheet=Image.new('RGB',(1536,768),(40,40,40));scene.render.resolution_x=384;scene.render.resolution_y=384
 import math
 for i in range(8):
  az=math.radians(i*45);direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN));camera.location=target+direction*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(views/f'view-{i}-textured.png');bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');tile=Image.new('RGB',im.size,(40,40,40));tile.paste(im,mask=im.getchannel('A'));sheet.paste(tile,((i%4)*384,(i//4)*384))
 sheet.save(dst/'textured.png');assert sha(source_model)==source_hash;write_json(dst/'preservation.json',dict(status='PASS exact known RGB and reopened geometry/UV/transforms; actual appearance review pending',source_model_sha256=source_hash,model_sha256=digest,geometry_uv_world_fingerprints=before,faces=receipts,source_model_unchanged=True,approval='Neither geometry nor texture approval inferred',limitations=['Hidden sides reuse only this sign’s reverse timber, so unobserved face-specific knots remain unknown.','Native shadow and canopy compositing remain separate.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
