"""Freeze a fitted anatomy with explicitly unapproved inherited path depth."""
import argparse,json,hashlib,shutil,math
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline

def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--geometry',type=Path,required=True);p.add_argument('--depth',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);sha=lambda x:hashlib.sha256(x.read_bytes()).hexdigest();fit=json.loads(a.fit.read_text());d=json.loads(a.depth.read_text());curve=d['continuous_height_curve'];z=CubicSpline(curve['times'],curve['heights'],bc_type='periodic');s=math.sin(math.radians(35));c=math.cos(math.radians(35));shutil.copyfile(a.geometry,a.output/'geometry.py')
 for row in fit['rows']:
  height=float(z(row['phase']));x,y=row['source']['alpha_centroid_display'];row['fixed_path_anchor_zup']=[x,-(y+c*height)/s,height]
 fit['geometry_helper_sha256']=sha(a.output/'geometry.py');fit['parent_fitted_motion_sha256']=sha(a.fit);fit['status']='PRIVATE_FOUR_WING_FIT_PENDING_SAVED_REVIEW_AND_CONTACT';dest=a.output/'fit.json';dest.write_text(json.dumps(fit,indent=2)+'\n');d.update(status='INHERITED_DEPTH_PLACEHOLDER_NO_FOUR_WING_CONTACT_APPROVAL',inherited_depth_sha256=sha(a.depth),source_fit_sha256=sha(dest),joined_fit_sha256=sha(dest),geometry_helper_sha256=sha(a.output/'geometry.py'));(a.output/'inputs.json').write_text(json.dumps(d,indent=2)+'\n')
if __name__=='__main__':main()
