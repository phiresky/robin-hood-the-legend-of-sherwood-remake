"""Verify saved wing vertices at every authored phase against independent rigid transforms."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
BASE=ROOT/'level-editor/work/active/croisement02';MODEL=BASE/'restart65-butterfly07-exact-model-review-v1/model.blend';FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json'
def rotation(axis,angle):
 a=math.radians(angle);c=math.cos(a);s=math.sin(a)
 return {'x':np.array([[1,0,0],[0,c,-s],[0,s,c]]),'y':np.array([[c,0,s],[0,1,0],[-s,0,c]]),'z':np.array([[c,-s,0],[s,c,0],[0,0,1]])}[axis]
def main():
 fit=json.loads(FIT.read_text());bpy.ops.wm.open_mainfile(filepath=str(MODEL));sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));basis=np.array([[1,0,0],[0,-sin,-cos],[0,-cos,sin]]);maximum=0
 for phase,row in enumerate(fit['rows']):
  bpy.context.scene.frame_set(2*phase+1);bpy.context.view_layer.update();p=row['parameters'];body=rotation('z',p[2])@rotation('y',p[1])@rotation('x',p[0])
  for side,sign in enumerate((-1,1)):
   wing=np.array(fit['fixed_geometry']['wing_outline']);wing[:,0]*=sign;wing=wing@rotation('y',-sign*p[3+side]).T;wing[:,0]+=fit['fixed_geometry']['wing_hinge_offsets'][side];expected=(wing@body.T+np.array([p[5],p[6],0]))@basis.T;ob=bpy.data.objects[f'Fixed wing {side}'];actual=np.array([list(ob.matrix_world@v.co) for v in ob.data.vertices]);error=float(np.max(np.abs(expected-actual)));assert error<1e-5,(phase,side,error);maximum=max(maximum,error)
 result=dict(status='PASS_ALL99_SAVED_RIG_KEY_POSES',model_sha256=hashlib.sha256(MODEL.read_bytes()).hexdigest(),fit_sha256=hashlib.sha256(FIT.read_bytes()).hexdigest(),wing_pose_checks=198,maximum_vertex_component_error=maximum,limits=['Exact authored key poses checked; intermediate curve interpolation and flight-path integration remain separate.'])
 out=BASE/'restart65-butterfly07-private-gallery-v1/saved-model-verification.json';assert not out.exists();out.write_text(json.dumps(result,indent=2)+'\n');print(result)
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
