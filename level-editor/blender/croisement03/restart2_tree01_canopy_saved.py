"""Review private inferred crown backs with retained wood; observed owner is separate."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/blender/croisement02')]
from render_slots import acquire,release
from tree_geometry import material,one_sided,replace_mesh
B=R/'level-editor/work/active/croisement03/restart2';I=B/'tree01-canopy-volume-cpu-v1';O=B/'tree01-canopy-volume-saved-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));EXTRA_BUILD=None;EXTRA_VERIFY=None
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def resources():
 assert shutil.disk_usage(R).free>=10*1024**3+32*1024**2
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def main():
 resources();acquire(slots=2)
 try:
  resources();assert not O.exists();O.mkdir();base=B/'tree01-saved-complete-wood-v2/worker.blend';basehash=sha(base);data=json.loads((I/'geometry.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(base));s=bpy.context.scene;bpy.context.preferences.filepaths.save_version=0
  obj=bpy.data.objects.new('Tree01 inferred crown backs only',bpy.data.meshes.new('Tree01 crown'));s.collection.objects.link(obj);mat=material('Tree01 own-native inferred gold leaf backs',I/'native-donor-atlas.png',False);mat['texture_provenance']='Exact own-native Arbre08 donor crops; inferred hidden leaf placement';nodes=mat.node_tree.nodes;links=mat.node_tree.links;tex=next(n for n in nodes if n.type=='TEX_IMAGE');emission=nodes.new('ShaderNodeEmission');links.new(tex.outputs['Color'],emission.inputs['Color']);transparent=nodes.new('ShaderNodeBsdfTransparent');mix=nodes.new('ShaderNodeMixShader');links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(transparent.outputs[0],mix.inputs[1]);links.new(emission.outputs[0],mix.inputs[2]);output_node=next(n for n in nodes if n.type=='OUTPUT_MATERIAL');links.new(mix.outputs[0],output_node.inputs['Surface']);one_sided(mat);replace_mesh(obj,data['vertices'],data['faces'],data['uvs'],[mat]);obj['observed_owner']=False;obj['scope']='Private inferred volume; native observed crown is intentionally absent from this scoped review'
  if EXTRA_BUILD is not None:EXTRA_BUILD(s)
  model=O/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<8*1024**2;digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene;obj=s.objects['Tree01 inferred crown backs only'];p=np.array([list(v.co)for v in obj.data.vertices]);error=float(np.max(np.abs(p-data['vertices'])));assert error<1e-4;assert np.ptp(p[:,1])>=np.ptp(p[:,0]);wood=[o for o in s.objects if o.name.startswith('Provisional Tree01 wood')];observed=[o for o in s.objects if o.name.startswith('Tree01 observed native')]
  if EXTRA_VERIFY is not None:EXTRA_VERIFY(s,O)
  for o in s.objects:
   if o.type=='MESH':o.hide_render=o!=obj and o not in wood and o not in observed
  s.render.engine='CYCLES';s.cycles.samples=16;s.cycles.transparent_max_bounces=256;s.cycles.use_denoising=False;s.cycles.seed=17;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=256;s.render.resolution_y=320;s.render.resolution_percentage=100;s.render.film_transparent=True;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.view_settings.view_transform='Standard'
  camera=bpy.data.objects.new('Crown review camera',bpy.data.cameras.new('Crown review camera'));s.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_end=10000;s.camera=camera;allpoints=np.array([list(o.matrix_world@v.co)for o in [obj,*wood,*observed]for v in o.data.vertices]);center=Vector((allpoints.min(0)+allpoints.max(0))/2);camera.data.ortho_scale=max(float(np.ptp(allpoints[:,0])),float(np.ptp(allpoints[:,1])),float(np.ptp(allpoints[:,2])))*.92;sheet=Image.new('RGB',(1024,640),'#333333')
  def render(angle,name):
   theta=math.radians(angle);direction=Vector((math.sin(theta)*C,-math.cos(theta)*C,S));camera.location=center+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(O/name);bpy.ops.render.render(write_still=True);return Image.open(s.render.filepath).convert('RGBA')
  for i in range(8):
   im=render(i*45,f'actual-{i*45:03}.png');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*256,i//4*320))
  sheet.save(O/'actual-eight.png');s.cycles.transparent_max_bounces=512;probe=render(180,'convergence-180-512.png');a=np.array(Image.open(O/'actual-180.png')).astype(int);b=np.array(probe).astype(int);delta=np.abs(a-b);convergence={'max_channel_difference':int(delta.max()),'changed_pixels':int(np.any(delta>0,axis=2).sum()),'dimensions':list(probe.size),'bounces':[256,512]}
  assert sha(model)==digest and sha(base)==basehash;assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<=32*1024**2;receipt={'status':'HOLD_INFERRED_CROWN_ONLY_VISUAL_REVIEW_REQUIRED','model_sha256':digest,'cpu_sha256':sha(I/'geometry.json'),'base_wood_sha256':basehash,'saved_vertex_error':error,'width':float(np.ptp(p[:,0])),'depth':float(np.ptp(p[:,1])),'convergence':convergence,'images':{p.name:sha(p)for p in O.glob('*.png')},'limits':['Native-facing inferred backs are intentionally invisible; no full observed crown appearance claim.','Shared native Arbre08 and static native crown source ownership must be integrated once, separately.','Bank context hidden in scoped shape review; final contact test remains.']};(O/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
