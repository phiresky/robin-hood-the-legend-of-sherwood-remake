"""Validate a private two-wing corridor against current receiver triangles under resource guards."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2';os.environ['OMP_NUM_THREADS']='2'
import argparse,json,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
import restart14_butterfly07_anatomy_contacts as audit

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists();mem={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert mem['MemAvailable']>=10*1024**3;assert shutil.disk_usage(ROOT).free>=10*1024**3;d=json.loads((a.candidate/'inputs.json').read_text());assert audit.reader.sha(a.candidate/'fit.json')==d['joined_fit_sha256'];helper=a.candidate/'geometry.py';assert audit.reader.sha(helper)==d['geometry_helper_sha256'];audit.OUT=a.output
 audit.main(a.candidate/'fit.json',continuous_height_curve=d['continuous_height_curve'],sweep_subdivisions=8,dense_pose_steps=4,geometry_helper=helper,geometry_helper_sha256=audit.reader.sha(helper),receiver_asset_ids=None,first_witness_only=True,transition_candidates={},expected_map_sha256=audit.reader.sha(audit.reader.LIB/'scenes/croisement02.rhlos-map.json'))
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
