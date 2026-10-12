"""Run explicit current-receiver conservative contact checks for a private four-wing path."""
import argparse,json,shutil,os,sys
os.environ["OPENBLAS_NUM_THREADS"]="2"
os.environ["OMP_NUM_THREADS"]="2"
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/"level-editor/refinement"))
from render_slots import acquire,release
import restart150_forehind_anatomy_contacts as audit

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--map-sha256',required=True);p.add_argument('--receivers',nargs='+',required=True);a=p.parse_args();mem={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert mem['MemAvailable']>=10*1024**3,'Receiver contact audit needs10GiB available';assert shutil.disk_usage(a.output.parent).free>=10*1024**3;assert not a.output.exists();d=json.loads((a.candidate/'inputs.json').read_text());assert audit.reader.sha(a.candidate/'fit.json')==d['joined_fit_sha256'];helper=a.candidate/'geometry.py';audit.OUT=a.output
 audit.main(a.candidate/'fit.json',continuous_height_curve=d['continuous_height_curve'],sweep_subdivisions=8,dense_pose_steps=4,geometry_helper=helper,geometry_helper_sha256=audit.reader.sha(helper),receiver_asset_ids=a.receivers,first_witness_only=True,transition_candidates={},expected_map_sha256=a.map_sha256)
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
