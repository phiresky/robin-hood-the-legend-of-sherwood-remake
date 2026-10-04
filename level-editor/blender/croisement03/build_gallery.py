"""Expose complete reviewed packets only; retain the complete unfinished source scope."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    groups=json.loads((OUT/'catalog.json').read_text())['groups'];missing=[];items=[]
    for group in groups:
        missing.append(dict(id=group['id'],name=group['name'],status='refinement in progress',reason='Geometry, exact source ownership and saved-material review remain unfinished.'))
    additional=[('timber-bridge','Timber Bridge'),('stream-fallen-log','Stream Fallen Log'),('painted-undergrowth','Painted Undergrowth and Supplemental Wood'),('ground-and-moving-water','Ground and Moving Water'),('mission-state-objects','Mission State Objects')]
    for slug,name in additional:missing.append(dict(id='croisement03-'+slug,name=name,status='source inventory complete; construction pending',reason='Absent or incomplete in the native obstacle reconstruction; independent ownership and state validation required.'))
    manifest=OUT/'review-candidates.json';manifest.write_text(json.dumps(dict(map='Croisement03',total_groups=len(groups),items=items,without_packets=missing,status_counts={'native obstacle parts inventoried':106,'native mask domains inventoried':131,'mission patch instances inventoried':101,'ready for approval':0}),indent=2)+'\n')
    build(manifest,OUT/'gallery',pending_only=True)
if __name__=='__main__':main()
