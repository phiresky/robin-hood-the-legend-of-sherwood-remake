"""Audit the selected improved butterfly branch against current static receivers."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json
from pathlib import Path
import restart14_butterfly07_anatomy_contacts as audit
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement'
def main():
 prior=json.loads((audit.B/'butterfly07-v3-global-corridor-v2-actual-audit/report.json').read_text());helper=Path(__file__).with_name('restart21_butterfly07_geometry_v2.py');audit.OUT=BASE/'restart67-butterfly07-selected-contacts-v1'
 # An explicit selected-branch pool excludes the unchosen mirrored hypothesis.
 audit.main(BASE/'restart66-butterfly07-global-bright-fit-v1/fit.json',transition_candidates={},continuous_height_curve=prior['continuous_height_curve'],sweep_subdivisions=16,dense_pose_steps=16,geometry_helper=helper,geometry_helper_sha256=audit.reader.sha(helper),receiver_asset_ids=None,first_witness_only=True,expected_map_sha256=audit.reader.sha(audit.reader.LIB/'scenes/croisement02.rhlos-map.json'))
if __name__=='__main__':main()
