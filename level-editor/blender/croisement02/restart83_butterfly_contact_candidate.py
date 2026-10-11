"""Bind own physical anatomy and a disclosed initial depth hypothesis to contact tests."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse
import json
from pathlib import Path
import numpy as np
import restart14_butterfly07_anatomy_contacts as audit
from restart21_butterfly07_geometry_v2 import fixed_geometry


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--fit',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--run',action='store_true')
    args=parser.parse_args();args.output.mkdir(exist_ok=False);fit=json.loads(args.fit.read_text());scale=fit['fixed_uniform_scale']
    proposal=audit.B/'footprint-path-proposal-v2/proposal.json';route=next(r for r in json.loads(proposal.read_text())['rows'] if r['sequence']==fit['animation_index'])
    heights=np.asarray(route['world_zup_knots'])[:,2];assert len(heights)==99
    geometry=fixed_geometry()
    for name in ('wing_outline','body_radii','wing_hinge_offsets'):geometry[name]=(np.asarray(geometry[name])*scale).tolist()
    geometry['inferred']='Own-source pose candidate with one scaled rest anatomy. Depth is an inherited initial footprint hypothesis, not observed altitude or approval.'
    helper=args.output/'geometry.py'
    helper.write_text('import numpy as np\nfrom scipy.spatial.transform import Rotation\n'+
        'SPEC='+repr(geometry)+'\nWING=np.array(SPEC["wing_outline"])\nRADII=np.array(SPEC["body_radii"])\nHINGES=SPEC["wing_hinge_offsets"]\n'+
        'BODY=np.array([[RADII[0]*np.sin(a)*np.cos(t),RADII[1]*np.cos(a),RADII[2]*np.sin(a)*np.sin(t)] for a in np.linspace(0,np.pi,9) for t in np.linspace(0,2*np.pi,17)[:-1]])\n'+
        'def fixed_geometry():return SPEC\n'+
        'def geometry(p):\n    rotation=Rotation.from_euler("xyz",p[:3],degrees=True);wings=[]\n    for side,(sign,angle) in enumerate([(-1,p[3]),(1,p[4])]):\n        points=WING.copy();points[:,0]*=sign;points=Rotation.from_euler("y",-sign*angle,degrees=True).apply(points);points[:,0]+=HINGES[side];wings.append(rotation.apply(points))\n    return rotation.apply(BODY),wings\n')
    fit['fixed_geometry']=geometry;fit['geometry_helper_sha256']=audit.reader.sha(helper)
    for row,z in zip(fit['rows'],heights):
        x,y=row['source']['alpha_centroid_display'];row['fixed_path_anchor_zup']=[x,-(y+audit.reader.COS*z)/audit.reader.SIN,float(z)]
    joined=args.output/'fit.json';joined.write_text(json.dumps(fit,indent=2)+'\n')
    curve=dict(times=list(range(100)),heights=[*heights.tolist(),float(heights[0])])
    input_report=dict(status='PRIVATE_INITIAL_PHYSICAL_PATH_NOT_APPROVED',source_fit_sha256=audit.reader.sha(args.fit),old_depth_proposal_sha256=audit.reader.sha(proposal),joined_fit_sha256=audit.reader.sha(joined),geometry_helper_sha256=audit.reader.sha(helper),continuous_height_curve=curve,limits=['Depth inherited as a private candidate only; earlier sampled footprint evidence is not a current contact result.','Own current source centroid and registration are preserved; display elevation is never treated as physical altitude.','Physical cubic depth and SLERP rotations are inferred between native held frames.','Current static receivers will be checked; moving mission states and visibility remain separate.'])
    (args.output/'inputs.json').write_text(json.dumps(input_report,indent=2)+'\n')
    if args.run:
        audit.OUT=args.output/'contacts'
        audit.main(joined,continuous_height_curve=curve,sweep_subdivisions=4,dense_pose_steps=4,geometry_helper=helper,geometry_helper_sha256=audit.reader.sha(helper),receiver_asset_ids=None,first_witness_only=True,transition_candidates={},expected_map_sha256=audit.reader.sha(audit.reader.LIB/'scenes/croisement02.rhlos-map.json'))


if __name__=='__main__':main()
