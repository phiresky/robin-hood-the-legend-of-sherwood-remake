"""Split source-fitted lobes into independent fore/hind wing pairs for cyclic fitting."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from restart106_butterfly_four_wing_probe import split_y

def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads(a.fit.read_text());spec=fit['fixed_geometry'];halves=[split_y(np.asarray(spec['wing_outline']),True),split_y(np.asarray(spec['wing_outline']),False)];spec['wing_regions']=[x.tolist() for x in halves];spec['articulation']='Four independent fore/hind hinges; inferred split at body midline. No source silhouette deformation or scale change.'
 helper=a.output/'geometry.py';helper.write_text("import numpy as np\nfrom scipy.spatial.transform import Rotation\nSPEC="+repr(spec)+"\nWING=np.array(SPEC['wing_outline'])\nREGIONS=[np.array(x) for x in SPEC['wing_regions']]\nHINGES=SPEC['wing_hinge_offsets']\nUNIT=np.array([[np.sin(a)*np.cos(t),np.cos(a),np.sin(a)*np.sin(t)] for a in np.linspace(0,np.pi,9) for t in np.linspace(0,2*np.pi,17)[:-1]])\nPARTS=[UNIT*np.array(p['radii'])+np.array(p['center']) for p in SPEC['body_parts']]\ndef body_components(p):return [Rotation.from_euler('xyz',p[:3],degrees=True).apply(x) for x in PARTS]\ndef geometry(p):\n rotation=Rotation.from_euler('xyz',p[:3],degrees=True);wings=[]\n for side,sign in enumerate((-1,1)):\n  for region,outline in enumerate(REGIONS):\n   x=outline.copy();x[:,0]*=sign;angle=p[3+side] if region==0 else p[7+side];x=Rotation.from_euler('y',-sign*angle,degrees=True).apply(x);x[:,0]+=HINGES[side];wings.append(rotation.apply(x))\n return np.vstack(body_components(p)),wings\n")
 for r in fit['rows']:r['parameters']=r['parameters'][:7]+r['parameters'][3:5]
 fit['status']='PRIVATE_FOUR_WING_SEED_REQUIRES_CYCLIC_FIT_AND_REVIEW';fit['parent_fit_sha256']=hashlib.sha256(a.fit.read_bytes()).hexdigest();fit['geometry_helper_sha256']=hashlib.sha256(helper.read_bytes()).hexdigest();(a.output/'fit.json').write_text(json.dumps(fit,indent=2)+'\n')
if __name__=='__main__':main()
