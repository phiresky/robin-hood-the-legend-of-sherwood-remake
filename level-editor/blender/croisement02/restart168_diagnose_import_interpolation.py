"""Separate imported translation and quaternion interpolation errors without rendering."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from scipy.spatial.transform import Rotation,Slerp
from scipy.interpolate import CubicSpline
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def main():
 b=ROOT/'level-editor/work/croisement02-refinement';fit=json.loads((b/'restart157-butterfly03-temporal-candidate-v1/fit.json').read_text());d=json.loads((b/'restart157-butterfly03-temporal-candidate-v1/inputs.json').read_text())['continuous_height_curve'];depth=CubicSpline(d['times'],d['heights'],bc_type='periodic');bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(b/'restart160-butterfly03-temporal-model-v1/butterfly03-fullpath.glb'));root=bpy.data.objects['Butterfly03 full inferred flight path'];S=math.sin(math.radians(35));C=math.cos(math.radians(35));basis=np.array([[1,0,0],[0,-S,-C],[0,-C,S]]);rows=[]
 for phase in [36,36.125,36.25,36.5,36.75,37]:
  i=int(phase);t=phase-i;a=fit['rows'][i];z=fit['rows'][(i+1)%99];pa=np.array(a['parameters']);pb=np.array(z['parameters']);p=pa*(1-t)+pb*t;rotation=Slerp([0,1],Rotation.from_euler('xyz',[pa[:3],pb[:3]],degrees=True))([t])[0];p[:3]=rotation.as_euler('xyz',degrees=True);screen=np.array(a['source']['alpha_centroid_display'])*(1-t)+np.array(z['source']['alpha_centroid_display'])*t;height=float(depth(phase));expected=np.array([screen[0],-(screen[1]+C*height)/S,height])+basis@np.r_[p[5:7],0];scene.frame_set(int(2*phase),subframe=(2*phase)%1);bpy.context.view_layer.update();actual=np.array(root.matrix_world.translation);actualrot=Rotation.from_matrix(np.array(root.matrix_world.to_3x3()));expectedrot=Rotation.from_matrix(basis@rotation.as_matrix()@basis.T);rows.append({'phase':phase,'actual_root':actual.tolist(),'expected_root':expected.tolist(),'translation_error':float(np.linalg.norm(actual-expected)),'rotation_error_degrees':float(np.degrees((actualrot.inv()*expectedrot).magnitude()))})
 out=b/'restart168-interpolation-diagnosis-v1';out.mkdir(exist_ok=False);(out/'report.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps(rows))
acquire(2)
try:main()
finally:release()
