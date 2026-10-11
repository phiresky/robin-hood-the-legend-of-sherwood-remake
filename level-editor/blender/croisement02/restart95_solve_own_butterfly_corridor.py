"""Choose a bounded periodic depth path inside exact candidate receiver exclusions."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1');os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,json,shutil
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
import restart26_butterfly07_global_corridor as solver
import restart14_butterfly07_anatomy_contacts as audit

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--envelope',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--audit',action='store_true');a=p.parse_args();a.output.mkdir(exist_ok=False);source=a.candidate/'fit.json';fit=json.loads(source.read_text());envelope=json.loads(a.envelope.read_text());assert envelope['fit_sha256']==audit.reader.sha(source);assert envelope['map_sha256']==audit.reader.sha(audit.reader.LIB/'scenes/croisement02.rhlos-map.json')
 n=len(envelope['records']);dt=99/n;old=np.array([r['fixed_path_anchor_zup'][2] for r in fit['rows']]);baseline=np.interp(np.arange(n)*dt,np.arange(100),np.r_[old,old[0]]);bands=[solver.base.free_bands(r['forbidden_height_bands']) for r in envelope['records']];selected=solver.choose_bands(bands,baseline,dt,seconds=180,display=True);continuous=solver.base.solve_bands(bands,selected['path'],baseline,dt) if selected['success'] else None
 report=dict(status='BOUNDED_CORRIDOR_REQUIRES_ACTUAL_AUDIT' if continuous and continuous['success'] else 'HOLD_NO_BOUNDED_CORRIDOR_FOUND',envelope_sha256=audit.reader.sha(a.envelope),parent_fit_sha256=audit.reader.sha(source),selection=selected,continuous=continuous,limits=['New depth is inferred; source projection and fixed anatomy unchanged.','Static receiver exclusions only; moving states and final appearance separate.','Solver timeout is not proof no corridor exists.']);(a.output/'solver.json').write_text(json.dumps(report,indent=2)+'\n')
 if not continuous or not continuous['success']:return
 curve=continuous['curve'];depth=CubicSpline(curve['times'],curve['heights'],bc_type='periodic');helper=a.output/'geometry.py';shutil.copyfile(a.candidate/'geometry.py',helper)
 for row in fit['rows']:
  x,y=row['source']['alpha_centroid_display'];z=float(depth(row['phase']));row['fixed_path_anchor_zup']=[x,-(y+audit.reader.COS*z)/audit.reader.SIN,z]
 joined=a.output/'fit.json';joined.write_text(json.dumps(fit,indent=2)+'\n');inputs=json.loads((a.candidate/'inputs.json').read_text());inputs.update(status='PRIVATE_ENVELOPE_CONSTRAINED_DEPTH_NOT_APPROVAL',continuous_height_curve=curve,joined_fit_sha256=audit.reader.sha(joined),geometry_helper_sha256=audit.reader.sha(helper),depth_solver_sha256=audit.reader.sha(a.output/'solver.json'),envelope_sha256=audit.reader.sha(a.envelope));(a.output/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
 if a.audit:
  audit.OUT=a.output/'contacts';audit.main(joined,continuous_height_curve=curve,sweep_subdivisions=4,dense_pose_steps=4,geometry_helper=helper,geometry_helper_sha256=audit.reader.sha(helper),receiver_asset_ids=None,first_witness_only=True,transition_candidates={},expected_map_sha256=envelope['map_sha256'])
if __name__=='__main__':main()
