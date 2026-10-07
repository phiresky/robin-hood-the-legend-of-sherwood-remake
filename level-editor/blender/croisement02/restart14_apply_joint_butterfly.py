"""Prepare or execute a bounded saved-model trial of frozen butterfly poses.

Preparation writes JSON only. Execution retains the conserved rest meshes and
fixed anatomical material, acquires the shared render lease, and reopens its
new model before reviewing it. No fitting is performed here.
"""
from pathlib import Path
import argparse,copy,hashlib,json,shutil,sys
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';SOURCE=BASE/'joint99-cpu-v2';OUT=BASE/'rig-joint-trial-v1'
REVIEW_PHASES=[0,2,18,27,28,29,30,47];MIN_FREE_GIB=10;MAX_OUTPUT_MIB=20
FROZEN_PROPOSAL='184b93b9ee7c822ce1aa90e2a1565be441db1ecec6953efbe778421738134ac1'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def prepare():
 proposal=json.loads((SOURCE/'joint-fit-proposal.json').read_text());parent=BASE/'rig-full-v2/fit.json';assert sha(parent)==proposal['source_fit_sha256'];packet=json.loads(parent.read_text());assert sha(BASE/'rig-full-v2/model.blend')==proposal['parent_model_sha256'];rows=proposal['selected']['rows'];assert len(rows)==99 and proposal['selected']['loop_pose_exact'];assert len(packet['source_clock'])==99
 # The root review binds the focused CPU proposal, not geometry or appearance.
 review=json.loads((SOURCE/'root-focused-review-v1.json').read_text());assert sha(SOURCE/'joint-fit-proposal.json')==review['evidence']['joint-fit-proposal.json']==FROZEN_PROPOSAL
 assert all(sha(Path(row['source']['source']))==row['source']['sha256'] for row in packet['poses']);assert sha(Path(packet['material_authority']['fixed_pattern_image']))==packet['material_authority']['fixed_pattern_sha256']
 for phase,(pose,new) in enumerate(zip(packet['poses'],rows)):
  assert pose['phase']==new['phase']==phase;pose['parameters']=copy.deepcopy(new['parameters']);pose.update(covered_source_centers=new['covered'],missing_source_centers=new['missing'],extra_centers=new['extra'])
 packet.update(status='FROZEN_CPU_POSES_PENDING_SAVED_MODEL_REVIEW',joint_cpu_proposal_sha256=sha(SOURCE/'joint-fit-proposal.json'),source_clock_unchanged=True)
 OUT.mkdir(exist_ok=True);fitbytes=(json.dumps(packet,indent=2)+'\n').encode()
 if (OUT/'fit.json').exists():assert (OUT/'fit.json').read_bytes()==fitbytes, 'Refuse a changed fit in an existing trial namespace'
 else:(OUT/'fit.json').write_bytes(fitbytes)
 summary={k:proposal['selected'][k] for k in ['covered','missing','extra','continuity_weight','max_body_step_degrees','mean_body_step_degrees','loop_pose_exact']};summary.update(frames=99,cycle_ticks=198,source_centers=sum(r['source_centers'] for r in rows),method='CPU analytical fit, not rendered parity');(OUT/'fit-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 command='/usr/bin/blender --background --threads 2 --python level-editor/blender/croisement02/restart14_apply_joint_butterfly.py -- --execute'
 plan={'status':'PREPARED_NOT_EXECUTED','fit_sha256':sha(OUT/'fit.json'),'proposal_sha256':FROZEN_PROPOSAL,'root_review_sha256':sha(SOURCE/'root-focused-review-v1.json'),'rest_mesh_parent_sha256':sha(BASE/'rig-v1/model.blend'),'fixed_material_authority':packet['material_authority'],'source_clock':packet['source_clock'],'minimum_free_gib':MIN_FREE_GIB,'maximum_trial_output_mib':MAX_OUTPUT_MIB,'render_size':[192,192],'samples':6,'threads':2,'shared_fifo_slots':4,'review_phases':REVIEW_PHASES,'native_rear_frames':198,'actual_solid_orbit_frames':len(REVIEW_PHASES)*16,'total_png_render_calls':198+len(REVIEW_PHASES)*16,'new_models':1,'command':command,'post_render_measurement':'python3 level-editor/blender/croisement02/restart14_butterfly_render_error.py rig-joint-trial-v1','limitations':['No fitting or propagation.','Original-camera view0 first in every orbit.','Only a private trial; root/user review and scene trajectory remain separate.','Refuse execution below10GiB; check free space before every frame and20MiB output budget.']};(OUT/'run-plan.json').write_text(json.dumps(plan,indent=2)+'\n');return plan

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--execute',action='store_true');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else None)
 if args.execute:
  assert shutil.disk_usage(BASE).free>MIN_FREE_GIB*1024**3, 'Execution blocked: below10GiB free'
  assert not (OUT/'model.blend').exists(), 'Recover and inspect any existing trial; never overwrite it automatically'
 plan=prepare()
 if not args.execute:print(json.dumps({'status':plan['status'],'command':plan['command'],'floor_gib':MIN_FREE_GIB,'cap_mib':MAX_OUTPUT_MIB},indent=2));return
 sys.path.insert(0,str(Path(__file__).resolve().parent));import restart14_render_butterfly_full as renderer
 renderer.OUT=OUT;renderer.acquire()
 try:
  shutil.copyfile(__file__,OUT/'pose-application-recipe.py');renderer.main(min_free_gib=MIN_FREE_GIB,review_phases=REVIEW_PHASES,output_budget_mib=MAX_OUTPUT_MIB,comparison_phases=REVIEW_PHASES,render_threads=2)
 finally:renderer.release()
if __name__=='__main__':main()
