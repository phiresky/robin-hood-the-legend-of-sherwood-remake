"""Private normal-only trunk shading trial; preserve all geometry and source data."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy
import numpy as np
from scipy.spatial import cKDTree
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
source=R/'tree08-wood-prototype-v21-native-margin/model.blend'
out=R/'tree08-wood-prototype-v22-normal-preview'
assert not out.exists()
def fingerprint(obj):
 m=obj.data
 return dict(vertices=[list(v.co) for v in m.vertices],faces=[list(p.vertices) for p in m.polygons],uv=[[list(x.uv) for x in u.data] for u in m.uv_layers],images={im.name:hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file})
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(source));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=obj.data
 original=fingerprint(obj);points=np.array([obj.matrix_world@v.co for v in mesh.vertices]);normals=np.array([v.normal[:] for v in mesh.vertices]);result=normals.copy()
 s,c=math.sin(math.radians(35)),math.cos(math.radians(35));native=-points[:,1]*s-points[:,2]*c
 def smooth(t):
  t=np.clip(t,0,1);return t*t*(3-2*t)
 weight=smooth((native-235)/20)*smooth((335-native)/20)*smooth((points[:,0]-520)/15)*smooth((620-points[:,0])/15)
 tree=cKDTree(points)
 for i in np.flatnonzero(weight):
  near=np.array(tree.query_ball_point(points[i],4.5));delta=points[near]-points[i]
  w=np.exp(-np.sum(delta*delta,axis=1)/8)*(normals[near]@normals[i]>.6)
  mean=(normals[near]*w[:,None]).sum(0);mean/=np.linalg.norm(mean)
  result[i]=normals[i]*(1-weight[i])+mean*weight[i];result[i]/=np.linalg.norm(result[i])
 mesh.normals_split_custom_set_from_vertices(result.tolist());mesh.update()
 out.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
 bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');assert original==fingerprint(obj)
 scene=bpy.context.scene;scene.render.engine='BLENDER_WORKBENCH';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.5,.5,.5);scene.display.shading.show_cavity=True
 camera_data=bpy.data.cameras.new('Original art camera first');camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);scene.camera=camera;camera_data.type='ORTHO';camera_data.ortho_scale=190;camera_data.clip_end=10000
 target=Vector((560,-650,115))
 for i in range(4):
  d=Vector((c*math.sin(i*math.tau/4),-c*math.cos(i*math.tau/4),s));camera.location=target+d*1800;camera.rotation_euler=(-d).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(out/f'close-{i}.png');bpy.ops.render.render(write_still=True)
 (out/'receipt.json').write_text(json.dumps(dict(status='PRIVATE_NORMAL_ONLY_VISUAL_TRIAL',source_sha256=sha(source),model_sha256=sha(out/'model.blend'),geometry_uv_and_packed_images_exact=True,custom_normal_vertices=int(np.count_nonzero(weight)),scope='Normals only; visual review pending. Geometric and receiver proofs inherit only because positions/faces/UV/images verified exact.'),indent=2)+'\n')
finally:release()
