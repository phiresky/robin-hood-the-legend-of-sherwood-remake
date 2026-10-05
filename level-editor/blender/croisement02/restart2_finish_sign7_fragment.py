"""Resolve the isolated movable phase26 fragment; retain planted-foot contact caveat."""
import sys,json
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS
from sign_rigid_depth import bend
from sign_source_raster import raster
from restart2_reproject_shrub57_fronts import main as reproject
from restart2_review_sign_shrub57 import main as joint
from restart2_check_sign_bend_ground_parity import main as ground

def main():
 base=OUT/'restart2-fence/shrub57-sign-bend-v7/model.blend';diagnostic=OUT/'restart2-fence/sign7-residual-depth-v2/report.json';data=json.loads(diagnostic.read_text());assert sha(base)==data['model_sha256']
 row=next(r for r in data['records'] if r['phase']==26 and r['pixel']==[60,275]);assert row['free_interval_behind_sign']>30 and 0<row['retreat_to_back']<1
 dest=OUT/'restart2-fence/shrub57-sign-bend-v8';dest.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(base));obj=bpy.data.objects['West Rock Foliage 57']
 before=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);uv=[tuple(x.uv) for x in obj.data.uv_layers['Foliage UV'].data]
 detail={'pixels':[{'source_pixel':row['pixel'],'blockers':[{'polygon':row['leaf']['face'],'required_retreat':row['retreat_to_back']+.1}]}]}
 deformation=bend(obj,detail,allowed_groups={row['leaf']['component']},field_radius=4)
 assert deformation['changed_components']==1 and deformation['unresolved_required_components']==0
 after=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);project=lambda a:np.column_stack((a[:,0],-SIN*a[:,1]-COS*a[:,2]));error=float(np.max(abs(project(before)-project(after))));assert error<.0002;assert uv==[tuple(x.uv) for x in obj.data.uv_layers['Foliage UV'].data]
 _,_,depth,_=raster(obj,[60,275,61,276],True);assert depth[0,0]<row['sign_back']-.1,(float(depth[0,0]),row['sign_back'])
 bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
 write_json(dest/'report.json',dict(status='Private one-fragment phase26 correction; fresh validation follows',model_sha256=sha(dest/'model.blend'),base_model_sha256=sha(base),diagnostic_sha256=sha(diagnostic),deformation=deformation,source_projection_max_error=error,uv_unchanged=True,phase26_source_pixel=[60,275],phase26_foliage_depth=float(depth[0,0]),phase26_sign_back_depth=row['sign_back'],contact_caveat='Four planted-post pixels have no positive exterior volume interval between complete post and bank (within0.001 ray unit). They remain explicitly disclosed, not buried or clipped.'))
 reproject(base_version=8,version=9)
 joint(version=9)
 ground(version=9)
 print('COMPLETE',OUT/'restart2-fence/shrub57-sign-bend-v9')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
