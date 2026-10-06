"""Review complete filled rig surfaces, with the native camera first."""
import sys,math,json
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart4_stump_final_contact import frame
from tree_geometry import RAY,SIN,COS
from bake_texture_candidate import pixels,array_hash

def main(key):
 w=OUT/f'restart5-initial-nets/texture-fill-v1/profile-{key}/experiment/unseen-complete-v1';h=sha(w/'worker.blend');out=w/'underside';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(w/'worker.blend'));scene=bpy.context.scene;proof=json.loads((w/'continuation.json').read_text());assert proof['model_sha256']==h
 for row in proof['objects']:
  obj=scene.objects[row['object']];mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'));node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);assert array_hash(pixels(node.image))==row['filled_rgb_sha256']
 objects=[o for o in scene.objects if o.type=='MESH'];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';sheet=Image.new('RGBA',(1024,1024));rows=[]
 for i,direction in enumerate([RAY,Vector((0,-COS,-SIN)),Vector((COS,0,-SIN)),Vector((0,COS,-SIN))]):
  cam=frame(scene,objects,direction,512,1.2);scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True);sheet.paste(Image.open(out/f'view-{i}.png').convert('RGBA'),((i%2)*512,(i//2)*512));rows.append(dict(view=i,camera=[list(r)for r in cam.matrix_world],ortho_scale=cam.data.ortho_scale))
 sheet.save(out/'sheet.png');write_json(out/'evidence.json',dict(model_sha256=h,views=rows,native_first=True,reopened_generated_RGBA_exact=True,scope='Native view followed by three below-ground inspection directions; no game floor occlusion, so all lower cord/loop/net faces are exposed.'));assert sha(w/'worker.blend')==h
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
