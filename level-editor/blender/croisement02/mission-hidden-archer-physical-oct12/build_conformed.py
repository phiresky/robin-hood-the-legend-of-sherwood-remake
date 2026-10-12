"""Apply measured local clearance to three private screen hypotheses."""
import json
import build_models as build
from render_slots import acquire,release

def main():
 path=build.q.OUT/'conformation-v2/depth-field.json';data=json.loads(path.read_text());build.DEST=build.q.OUT/'geometry-v3-conformed';build.PROFILES={1,3,4};build.CLEARANCE=data['rows'];assert all(r['maximum_advance']<=12.401 for r in data['rows'].values())
 build.main();report=build.DEST/'report.json';r=json.loads(report.read_text());r['clearance_field']=build.q.pin(path);r['clearance_note']='Measured near-surface depth plus inferred clearance and smoothed transition. Canopy faces do not establish attachment. Full volume recheck required.';report.write_text(json.dumps(r,indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
