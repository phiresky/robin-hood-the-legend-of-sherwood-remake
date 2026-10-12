"""Build a private receiver-clearance revision, retaining each observed source pose."""
import sys,json
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import build_models as build
from render_slots import acquire,release

def main():
 path=build.q.OUT/'conformation-v4-cell-bounds/depth-field.json'
 data=json.loads(path.read_text())
 assert data['status']=='PRIVATE_CLEARANCE_HYPOTHESIS_REQUIRES_MODEL_AND_FULL_RECHECK'
 assert all(r['maximum_added_advance']<=8 for r in data['rows'].values()),'Inspect larger inferred displacement before construction'
 build.DEST=build.q.OUT/'geometry-v4-cell-conformed';build.PROFILES={1,3,4};build.CLEARANCE=data['rows']
 build.main()
 for n in sorted(build.PROFILES):
  build.guard();folder=build.DEST/f'profile-{n:02}';model=folder/'model.blend';before=build.q.pin(model);build.bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in build.bpy.context.scene.objects if o.type=='MESH']
  for phase in ['transition-0','transition-1']:
   obj=next(o for o in objects if o['source_phase']==phase)
   for other in objects:other.hide_render=other!=obj;other.hide_viewport=other!=obj
   out=folder/phase;out.mkdir();build.render(obj,out)
  assert build.q.pin(model)==before
 pathout=build.DEST/'report.json';report=json.loads(pathout.read_text());report['clearance_field']=build.q.pin(path);report['scope']='Full native pose geometry retained; inferred local depth now bounds entire previous contact triangle portions. Full contact audit and joined coverage still required.'
 pathout.write_text(json.dumps(report,indent=2)+'\n')
 selection=build.q.OUT/'conformed-v4-selected.json';assert not selection.exists();selection.write_text(json.dumps(dict(report=build.q.pin(pathout),models=[r['model']for r in report['profiles']],status='PRIVATE_ALL_POSE_REVIEW_PENDING'),indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
