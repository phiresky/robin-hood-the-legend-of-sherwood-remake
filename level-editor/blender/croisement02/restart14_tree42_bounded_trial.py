"""Build one capped physical trial from frozen CPU field coefficients."""
from pathlib import Path
import sys,json,hashlib,shutil,resource
import numpy as np
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from refinement_workspace import _geometry
from tree_geometry import SIN,COS
from restart14_tree42_dense_prototype import sample_field
BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=BASE/'tree42-motion-v5';MIB=2**20;FLOOR=10*2**30;CAP=256*MIB;MODEL_CAP=80*MIB;sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def usage():return sum(p.stat().st_size for p in DEST.rglob('*')if p.is_file())+sum(p.stat().st_size for p in BASE.glob('tree42-v5-*.log'))
def guard(expected=4*MIB):
 free=shutil.disk_usage(BASE).free;used=usage();assert free-expected>=FLOOR,('Bounded trial free-space floor',free,expected);assert used+expected<=CAP,('Bounded trial output cap',used,expected)
 resume=DEST/'resume-inputs.json'
 if resume.exists():
  cap=json.loads(resume.read_text());assert used+expected<=cap['existing_trial_bytes']+cap['additional_output_cap_bytes'],('Resume output cap',used,expected)
def finish(name):
 guard(MIB);(DEST/f'{name}-resources.json').write_text(json.dumps({'peak_rss_kib':resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,'threads':2,'output_bytes':usage(),'free_bytes':shutil.disk_usage(BASE).free,'output_cap_bytes':CAP,'model_cap_bytes':MODEL_CAP,'free_floor_bytes':FLOOR},indent=2)+'\n')
def install_write_guards():
 from PIL import Image
 save=Image.Image.save;write=Path.write_text
 def checked_save(self,*args,**kwargs):guard();return save(self,*args,**kwargs)
 def checked_write(self,*args,**kwargs):guard(MIB);return write(self,*args,**kwargs)
 Image.Image.save=checked_save;Path.write_text=checked_write
 bpy.context.preferences.filepaths.save_version=0
 for scene in bpy.data.scenes:scene.render.threads_mode='FIXED';scene.render.threads=2

def main():
 guard(MODEL_CAP);DEST.mkdir(exist_ok=False);source=BASE/'tree42-motion-v4/prototype.blend';assert sha(source)=='ed90774d18790d35b23b2d20941c609b2420004ee4b4a1b293872ba934150376';cp=BASE/'tree42-anchor-field-v2/report.json';assert sha(cp)=='e7ac68764419139f9b4f44b9b59f6a58f33ae60417bd99235dbaab4f589bcc88';spec=json.loads(cp.read_text());parent=BASE/'tree42-coherent-correspondence-v2/flows.npz';assert sha(parent)==spec['parent_field_sha256'];packet=np.load(parent);old=packet['flow'].astype(float);points=np.array(spec['source_centers_pixel_index']);coeff=np.array(spec['correction_coefficients']);yy,xx=np.mgrid[:288,:342];basis=np.exp(-((xx[:,:,None]-points[:,0])**2+(yy[:,:,None]-points[:,1])**2)/(2*spec['radius_pixels']**2));field=old+np.array(spec['phase_weights'])[:,None,None,None]*np.einsum('hwk,kc->hwc',basis,coeff)[None];assert np.array_equal(field[0],old[0]);bpy.ops.wm.open_mainfile(filepath=str(source));install_write_guards();scene=bpy.context.scene;scene.frame_set(1);objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-42'];guards={o.name:_geometry(o,protect_appearance=True)for o in objects};crown=next(o for o in objects if o.get('projection_component')=='crown');initial=np.array([v.co[:]for v in crown.data.vertices]);world=np.array([crown.matrix_world@Vector(v)for v in initial]);native=np.c_[world[:,0],-world[:,1]*SIN-world[:,2]*COS];inverse=crown.matrix_world.inverted().to_3x3();keys=crown.data.shape_keys.key_blocks;assert len(keys)==14;stats=[]
 for phase in range(1,14):
  delta=sample_field(field[phase],native,packet['bbox']);disp=np.c_[delta[:,0],-delta[:,1]*SIN,-delta[:,1]*COS];local=np.array([inverse@Vector(v)for v in disp]);keys[phase].data.foreach_set('co',(initial+local).reshape(-1));stats.append({'phase':phase,'maximum_vertex_motion':float(np.linalg.norm(local,axis=1).max())})
 bpy.context.view_layer.update();assert guards=={o.name:_geometry(o,protect_appearance=True)for o in objects};guard(MODEL_CAP);bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'prototype.blend'));assert(DEST/'prototype.blend').stat().st_size<=MODEL_CAP;bpy.ops.wm.open_mainfile(filepath=str(DEST/'prototype.blend'));scene=bpy.context.scene;scene.frame_set(1);assert guards=={n:_geometry(scene.objects[n],protect_appearance=True)for n in guards};assert sha(source)=='ed90774d18790d35b23b2d20941c609b2420004ee4b4a1b293872ba934150376';(DEST/'report.json').write_text(json.dumps({'status':'BOUNDED_PHYSICAL_TRIAL_PENDING_REVIEW','prototype_sha256':sha(DEST/'prototype.blend'),'source_worker_sha256':sha(source),'field_coefficients_sha256':sha(cp),'static_basis_appearance_uv_geometry_exact':True,'wood_fixed':True,'phase_motion':stats,'limits':['No texture/alpha edits or additional geometry; only existing crown shape-key poses updated.','Seven source-supported cluster constraints do not prove exact all-phase coverage.','Source4tick holds, phase0 and loop preserved.']},indent=2)+'\n');finish('build')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
