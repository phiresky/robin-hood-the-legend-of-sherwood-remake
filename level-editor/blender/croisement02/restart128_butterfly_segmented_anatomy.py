"""Propose separate head, thorax and abdomen with broader curved wing lobes."""
import argparse,json
from pathlib import Path
import numpy as np
from hashlib import sha256

def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads(a.fit.read_text());old=np.array(fit['fixed_wing_outline_scaled']);scale=fit['fixed_uniform_scale']
 def arc(start,mid,end,count):
  control=2*mid-(start+end)/2
  return np.array([(1-t)**2*start+2*t*(1-t)*control+t*t*end for t in np.linspace(0,1,count)])
 wing=np.vstack([old[0],arc(old[1],old[2],old[3],9),arc(old[4],old[5],old[6],11)])
 parts=[dict(name='head',center=[0,-3.2*scale,0],radii=[.42*scale,.55*scale,.42*scale]),dict(name='neck',center=[0,-2.15*scale,0],radii=[.18*scale,.62*scale,.18*scale]),dict(name='thorax',center=[0,-.8*scale,0],radii=[.5*scale,.9*scale,.5*scale]),dict(name='abdomen',center=[0,1.5*scale,0],radii=[.33*scale,1.7*scale,.33*scale])]
 spec=dict(wing_outline=wing.tolist(),wing_hinge_offsets=[-.28*scale,.28*scale],body_parts=parts,inferred='Private source-guided hypothesis. Separate head/neck/thorax/tapered abdomen replace the short capsule. Broader curved upper/lower lobes pass through previous source-fitted contour extrema; dimensions and hidden depth require refitting and review.')
 helper=a.output/'geometry.py';helper.write_text('import numpy as np\nfrom scipy.spatial.transform import Rotation\nSPEC='+repr(spec)+'\nWING=np.array(SPEC["wing_outline"])\nHINGES=SPEC["wing_hinge_offsets"]\nUNIT=np.array([[np.sin(a)*np.cos(t),np.cos(a),np.sin(a)*np.sin(t)] for a in np.linspace(0,np.pi,9) for t in np.linspace(0,2*np.pi,17)[:-1]])\nPARTS=[UNIT*np.array(p["radii"])+np.array(p["center"]) for p in SPEC["body_parts"]]\ndef fixed_geometry():return SPEC\ndef body_components(p):return [Rotation.from_euler("xyz",p[:3],degrees=True).apply(x) for x in PARTS]\ndef geometry(p):\n rotation=Rotation.from_euler("xyz",p[:3],degrees=True);wings=[]\n for side,sign in enumerate((-1,1)):\n  x=WING.copy();x[:,0]*=sign;x=Rotation.from_euler("y",-sign*p[3+side],degrees=True).apply(x);x[:,0]+=HINGES[side];wings.append(rotation.apply(x))\n return np.vstack(body_components(p)),wings\n')
 fit['fixed_geometry']=spec;fit['fixed_wing_outline_scaled']=wing.tolist();fit['geometry_helper_sha256']=sha256(helper.read_bytes()).hexdigest();fit['status']='PRIVATE_SEGMENTED_ANATOMY_SEED_METRICS_NOT_VALID_UNTIL_REFIT';fit['segmented_anatomy_parent_sha256']=sha256(a.fit.read_bytes()).hexdigest();(a.output/'fit.json').write_text(json.dumps(fit,indent=2)+'\n');(a.output/'scope.json').write_text(json.dumps(dict(status=fit['status'],spec=spec,limits=['Body part positions and shape are provisional, not established by gold-pixel color alone.','Existing pose/coverage metrics are stale after this shape change; component-aware source evaluation is required.','Global convex hull of all body points is conservative only for contact; source fitting and rendering must use the separate surfaces.','No texture synthesis or approval.']),indent=2)+'\n')
if __name__=='__main__':main()
